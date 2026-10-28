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
        branches.retain(|v| v.get("else").is_some() || selects(&v["if"], op) != Some(false));
    }
    value
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
    match outer.get_mut("allOf").and_then(Value::as_array_mut) {
        Some(branches) => branches.push(selected),
        None => outer["allOf"] = json!([selected]),
    };
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
    let params = matching
        .into_iter()
        .next()
        .map(|v| v["then"]["properties"]["params"].clone())
        .ok_or("operation parameters")?;
    bundle(
        source,
        json!({"type":"object","additionalProperties":false,"required":["host","params"],"properties":{"host":source["properties"]["host"],"params":params,"if_boot_id":source["properties"]["if_boot_id"]}}),
    )
}
