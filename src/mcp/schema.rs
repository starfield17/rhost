//! Conservative operation specialization; the versioned schemas remain authoritative.
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
fn selects(condition: &Value, op: &str) -> Option<bool> {
    let rule = condition.get("properties")?.get("operation")?;
    if let Some(value) = rule.get("const") {
        return Some(value == op);
    }
    rule.get("enum")?
        .as_array()
        .map(|values| values.iter().any(|v| v == op))
}
fn specialize(mut value: Value, op: &str) -> Value {
    value["properties"]["operation"] = json!({"const":op});
    if let Some(branches) = value.get_mut("allOf").and_then(Value::as_array_mut) {
        let mut selected = Vec::new();
        for mut branch in std::mem::take(branches) {
            match selects(&branch["if"], op) {
                Some(false) => {
                    if let Some(otherwise) = branch.get("else") {
                        selected.push(otherwise.clone());
                    }
                }
                Some(true) => {
                    if let Some(properties) = branch["if"]["properties"].as_object_mut() {
                        properties.remove("operation");
                    }
                    if branch["if"] == json!({"properties":{}}) {
                        if let Some(then) = branch.get("then") {
                            selected.push(then.clone());
                        }
                    } else {
                        selected.push(branch);
                    }
                }
                None => selected.push(branch),
            }
        }
        *branches = selected;
    }
    value
}

/// Intersect two closed envelope objects only when their allowed keys agree.
fn envelope(mut outer: Value, selected: Value) -> Value {
    let same_keys = outer["properties"]
        .as_object()
        .zip(selected["properties"].as_object())
        .is_some_and(|(a, b)| a.keys().eq(b.keys()));
    let known_shape = selected.as_object().is_some_and(|m| {
        m.keys().all(|k| {
            matches!(
                k.as_str(),
                "type"
                    | "required"
                    | "properties"
                    | "additionalProperties"
                    | "allOf"
                    | "$defs"
                    | "definitions"
            )
        })
    });
    if !same_keys
        || !known_shape
        || outer["type"] != selected["type"]
        || outer["additionalProperties"] != false
        || selected["additionalProperties"] != false
    {
        match outer.get_mut("allOf").and_then(Value::as_array_mut) {
            Some(branches) => branches.push(selected),
            None => outer["allOf"] = json!([selected]),
        }
        return outer;
    }
    for (name, rule) in selected["properties"].as_object().into_iter().flatten() {
        let current = &outer["properties"][name];
        outer["properties"][name] = if current == rule || rule == &json!({}) {
            current.clone()
        } else if current == &json!({}) {
            rule.clone()
        } else {
            json!({"allOf":[current,rule]})
        };
    }
    if let (Some(required), Some(extra)) = (
        outer["required"].as_array_mut(),
        selected["required"].as_array(),
    ) {
        for field in extra {
            if !required.contains(field) {
                required.push(field.clone());
            }
        }
    }
    if let Some(extra) = selected["allOf"].as_array() {
        match outer.get_mut("allOf").and_then(Value::as_array_mut) {
            Some(branches) => branches.extend(extra.iter().cloned()),
            None => outer["allOf"] = extra.clone().into(),
        }
    }
    outer
}
struct Bundle<'a> {
    source: &'a Value,
    names: BTreeMap<String, String>,
    definitions: Map<String, Value>,
}
impl Bundle<'_> {
    fn visit(&mut self, value: &mut Value) -> Result<(), String> {
        match value {
            Value::Array(values) => {
                for v in values {
                    self.visit(v)?;
                }
            }
            Value::Object(map) => {
                if map.contains_key("$dynamicRef")
                    || map.contains_key("$anchor")
                    || map.contains_key("$dynamicAnchor")
                {
                    return Err("unsupported schema anchor".into());
                }
                if map.contains_key("$id") {
                    return Err("nested schema identity is unsupported".into());
                }
                map.remove("$defs");
                map.remove("definitions");
                if let Some(reference) = map.get_mut("$ref") {
                    let original = reference.as_str().ok_or("invalid reference")?.to_string();
                    if !original.starts_with("#/") {
                        return Err("non-pointer schema reference".into());
                    }
                    let name = if let Some(name) = self.names.get(&original) {
                        name.clone()
                    } else {
                        let name = format!("d{}", self.names.len());
                        self.names.insert(original.clone(), name.clone());
                        let mut target = self
                            .source
                            .pointer(&original[1..])
                            .ok_or("missing schema reference")?
                            .clone();
                        self.visit(&mut target)?;
                        self.definitions.insert(name.clone(), target);
                        name
                    };
                    *reference = format!("#/$defs/{name}").into();
                }
                for v in map.values_mut() {
                    self.visit(v)?;
                }
            }
            _ => (),
        }
        Ok(())
    }
}
fn bundle(source: &Value, mut selected: Value) -> Result<Value, String> {
    let mut b = Bundle {
        source,
        names: BTreeMap::new(),
        definitions: Map::new(),
    };
    if let Some(map) = selected.as_object_mut() {
        map.remove("$id");
    }
    b.visit(&mut selected)?;
    if !b.definitions.is_empty() {
        selected["$defs"] = Value::Object(b.definitions);
    }
    Ok(selected)
}
pub fn output(source: &Value, op: &str) -> Result<Value, String> {
    let branches = ["legacy", "native"]
        .into_iter()
        .filter_map(|name| {
            let branch = &source["$defs"][name];
            branch["properties"]["operation"]["enum"]
                .as_array()
                .is_some_and(|ops| ops.iter().any(|v| v == op))
                .then_some(branch)
        })
        .collect::<Vec<_>>();
    if branches.len() != 1 {
        return Err("operation must select exactly one result branch".into());
    }
    // The outer closed envelope constraints are retained as well as the chosen branch.
    let mut outer = source.clone();
    outer
        .as_object_mut()
        .ok_or("result object")?
        .remove("oneOf");
    outer = specialize(outer, op);
    let selected = specialize(branches[0].clone(), op);
    outer = envelope(outer, selected);
    bundle(source, outer)
}
pub fn input(source: &Value, op: &str) -> Result<Value, String> {
    let branches = source["allOf"].as_array().ok_or("input branches")?;
    if branches
        .iter()
        .any(|v| v.get("else").is_some() || selects(&v["if"], op).is_none())
    {
        return Err("unsupported generic input condition".into());
    }
    let matching = branches
        .iter()
        .filter(|v| selects(&v["if"], op) == Some(true))
        .collect::<Vec<_>>();
    if matching.len() != 1 {
        return Err("operation parameters must have one branch".into());
    }
    let branch = matching.into_iter().next().ok_or("operation parameters")?;
    let mut properties = Map::new();
    for field in ["host", "params", "if_boot_id"] {
        let value = branch["then"]["properties"]
            .get(field)
            .unwrap_or(&source["properties"][field]);
        if value != &Value::Bool(false) {
            properties.insert(field.into(), value.clone());
        }
    }
    let mut required = vec![Value::String("params".into())];
    if branch["then"]["required"]
        .as_array()
        .is_some_and(|values| values.iter().any(|v| v == "host"))
    {
        required.push(Value::String("host".into()));
    }
    bundle(
        source,
        json!({"type":"object","additionalProperties":false,"required":required,"properties":properties}),
    )
}
