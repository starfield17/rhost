//! Closed public request shapes, parsed before SSH or helper preparation.
use crate::helper::Writer;
use crate::remote::Error;
use serde::Deserialize;
use std::collections::BTreeMap;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Namespace {
    generation: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Argv {
    argv: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Shell {
    interpreter: String,
    script: String,
    login: bool,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Program {
    Argv(Argv),
    Shell(Shell),
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Environment {
    inherit: bool,
    #[serde(deserialize_with = "unique_environment")]
    values: BTreeMap<String, String>,
}
fn unique_environment<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<String, String>, D::Error> {
    struct Unique;
    impl<'de> serde::de::Visitor<'de> for Unique {
        type Value = BTreeMap<String, String>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("unique environment names")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<Self::Value, A::Error> {
            let mut values = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, String>()? {
                if values.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom("duplicate environment name"));
                }
            }
            Ok(values)
        }
    }
    d.deserialize_map(Unique)
}
fn nonnull_deadline<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    u64::deserialize(d).map(Some)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Start {
    generation: String,
    key: String,
    expires_unix: u64,
    cwd: String,
    program: Program,
    environment: Environment,
    #[serde(default)]
    stdin_base64: String,
    #[serde(default)]
    wait_ms: u32,
    #[serde(default, deserialize_with = "nonnull_deadline")]
    deadline_ms: Option<u64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Poll {
    generation: String,
    run_id: String,
    #[serde(default)]
    wait_ms: u32,
    #[serde(default)]
    stdout_since: u64,
    #[serde(default)]
    stderr_since: u64,
    #[serde(default = "page")]
    max_bytes: u32,
}
fn page() -> u32 {
    4096
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Signal {
    generation: String,
    run_id: String,
    signal: String,
}
pub struct Prepared {
    pub host: String,
    pub kind: u32,
    pub body: Writer,
    pub wait_ms: u32,
}
fn invalid(message: &str) -> Error {
    Error::new("CONFIG_INVALID", message).with_recovery(crate::domain::Recovery::invalid())
}
fn parse<T: for<'a> Deserialize<'a>>(raw: &str) -> Result<T, Error> {
    serde_json::from_str(raw).map_err(|e| invalid(&e.to_string()))
}
pub fn generation(value: &str) -> Result<(), Error> {
    if value.len() != 32
        || !value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(invalid("invalid namespace generation"));
    }
    Ok(())
}
pub fn key(value: &str) -> Result<u64, Error> {
    let (expiry, random) = value
        .split_once('-')
        .ok_or_else(|| invalid("invalid request key"))?;
    let seconds = expiry
        .parse::<u64>()
        .map_err(|_| invalid("invalid key expiry"))?;
    if seconds == 0
        || expiry.starts_with('0')
        || expiry.len() > 12
        || random.len() != 64
        || !random
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(invalid("invalid request key"));
    }
    Ok(seconds)
}
pub fn prepare(operation: &str, host: &str, params: &str) -> Result<Prepared, Error> {
    let mut body = Writer::default();
    let mut wait_ms = 0;
    let (kind, _operation) = match operation {
        "metadata" => {
            let _: Empty = parse(params)?;
            (8, "metadata")
        }
        "helper.prepare" => {
            let _: Empty = parse(params)?;
            (7, "helper.prepare")
        }
        "run.start" => {
            let p: Start = parse(params)?;
            generation(&p.generation)?;
            if key(&p.key)? != p.expires_unix
                || p.cwd.is_empty()
                || p.wait_ms > 300000
                || p.deadline_ms == Some(0)
                || p.deadline_ms.is_some_and(|n| n > 604800000)
            {
                return Err(invalid("invalid Run expiry, context or budget"));
            }
            body.text(&p.generation)?;
            body.text(&p.key)?;
            body.long(p.expires_unix);
            body.text(&p.cwd)?;
            match p.program {
                Program::Argv(p) => {
                    if p.argv.is_empty() || p.argv[0].is_empty() || p.argv.len() > 4096 {
                        return Err(invalid("argv requires a nonempty program"));
                    }
                    body.byte(1);
                    body.int(p.argv.len() as u32);
                    for arg in p.argv {
                        body.text(&arg)?;
                    }
                }
                Program::Shell(p) => {
                    if !p.interpreter.starts_with('/') || p.script.trim().is_empty() {
                        return Err(invalid(
                            "shell requires absolute interpreter and nonempty script",
                        ));
                    }
                    body.byte(2);
                    body.text(&p.interpreter)?;
                    body.text(&p.script)?;
                    body.byte(u8::from(p.login));
                }
            }
            body.byte(u8::from(p.environment.inherit));
            if p.environment.values.len() > 4096 {
                return Err(invalid("too many environment values"));
            }
            body.int(p.environment.values.len() as u32);
            for (name, value) in p.environment.values {
                if !crate::shell::is_valid_env_key(&name) {
                    return Err(invalid("invalid environment name"));
                }
                body.text(&name)?;
                body.text(&value)?;
            }
            if p.stdin_base64.contains(['\r', '\n']) {
                return Err(invalid("stdin requires unwrapped base64"));
            }
            let input = crate::base64::decode(&p.stdin_base64)
                .ok_or_else(|| invalid("invalid stdin base64"))?;
            body.bytes(&input)?;
            body.int(p.wait_ms);
            body.long(p.deadline_ms.unwrap_or(0));
            wait_ms = p.wait_ms;
            (2, "run.start")
        }
        "run.poll" => {
            let p: Poll = parse(params)?;
            generation(&p.generation)?;
            key(&p.run_id)?;
            if p.wait_ms > 300000 || p.max_bytes == 0 || p.max_bytes > 1048576 {
                return Err(invalid("invalid poll budget"));
            }
            body.text(&p.generation)?;
            body.text(&p.run_id)?;
            body.int(p.wait_ms);
            body.long(p.stdout_since);
            body.long(p.stderr_since);
            body.int(p.max_bytes);
            wait_ms = p.wait_ms;
            (3, "run.poll")
        }
        "run.list" | "run.prune" => {
            let p: Namespace = parse(params)?;
            generation(&p.generation)?;
            body.text(&p.generation)?;
            if operation == "run.list" {
                (4, "run.list")
            } else {
                (6, "run.prune")
            }
        }
        "run.signal" => {
            let p: Signal = parse(params)?;
            generation(&p.generation)?;
            key(&p.run_id)?;
            body.text(&p.generation)?;
            body.text(&p.run_id)?;
            body.byte(match p.signal.as_str() {
                "INT" => 1,
                "TERM" => 2,
                "KILL" => 3,
                _ => return Err(invalid("signal must be INT, TERM or KILL")),
            });
            (5, "run.signal")
        }
        _ => return Err(Error::new("USAGE_ERROR", "unknown structured operation")),
    };
    if body.0.len() > crate::helper::codec::LIMIT {
        return Err(invalid("encoded request exceeds 8 MiB"));
    }
    Ok(Prepared {
        host: host.to_string(),
        kind,
        body,
        wait_ms,
    })
}
