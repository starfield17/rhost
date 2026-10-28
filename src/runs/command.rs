//! Thin CLI construction of the same structured request used by agents.
use crate::cli::{FlagSpec, JSON};
use serde_json::{Value, json};
use std::collections::BTreeMap;
pub static FLAGS: &[FlagSpec] = &[
    JSON,
    FlagSpec::value("from", false, "JSON request file, or - for stdin"),
    FlagSpec::value("generation", true, "remote namespace generation"),
    FlagSpec::value("key", true, "caller-owned expiring idempotency key"),
    FlagSpec::value("cwd", true, "remote working directory; default ~"),
    FlagSpec::value("command", true, "exact shell program"),
    FlagSpec::value("argv-json", true, "JSON array of literal command arguments"),
    FlagSpec::value("shell", true, "absolute interpreter; default /bin/sh"),
    FlagSpec::long("login", "request a login shell"),
    FlagSpec::long("inherit-env", "inherit the remote invocation environment"),
    FlagSpec::value("env", false, "explicit KEY=VALUE; repeatable"),
    FlagSpec::value("wait", true, "local observation budget; default 0"),
    FlagSpec::value("deadline", true, "remote execution deadline"),
    FlagSpec::value("stdout-since", true, "stdout source-byte cursor"),
    FlagSpec::value("stderr-since", true, "stderr source-byte cursor"),
    FlagSpec::value("max-bytes", true, "per-stream page budget; default 4096"),
    FlagSpec::value("sig", true, "INT, TERM or KILL"),
];
pub enum Input {
    Document(String),
    File(String),
}
pub struct Invocation {
    pub input: Result<Input, String>,
    pub json: bool,
    pub help: bool,
    pub help_text: String,
}
fn allowed(group: &str, leaf: Option<&str>) -> &'static [&'static str] {
    match (group, leaf) {
        ("request", _) => &["json", "from"],
        ("helper", _) => &["json"],
        ("run", Some("start")) => &[
            "json",
            "generation",
            "key",
            "cwd",
            "command",
            "argv-json",
            "shell",
            "login",
            "inherit-env",
            "env",
            "wait",
            "deadline",
        ],
        ("run", Some("poll")) => &[
            "json",
            "generation",
            "wait",
            "stdout-since",
            "stderr-since",
            "max-bytes",
        ],
        ("run", Some("signal")) => &["json", "generation", "sig"],
        ("run", Some("list" | "prune")) => &["json", "generation"],
        _ => &[],
    }
}
pub fn help(group: &str, leaf: Option<&str>) -> String {
    let mut text = match group {
        "request" => "rhost request --from <file|->\n".to_owned(),
        "helper" => "rhost helper <prepare|status> <host>\n".to_owned(),
        _ => "rhost run <start|poll|list|signal|prune> <host> [run_id] [flags]\n".to_owned(),
    };
    let allowed = allowed(group, leaf);
    for flag in FLAGS {
        if !allowed.is_empty() && !allowed.contains(&flag.name) {
            continue;
        }
        text.push_str(&format!("  --{:<18} {}\n", flag.name, flag.help));
    }
    text
}
fn milliseconds(raw: Option<&str>) -> Result<u64, String> {
    match raw {
        None => Ok(0),
        Some(raw) => {
            let duration = crate::cli::parse_duration(raw).ok_or("invalid duration")?;
            u64::try_from(duration)
                .map(|n| n.div_ceil(1_000_000))
                .map_err(|_| "duration exceeds supported range".into())
        }
    }
}
pub fn parse(group: &str, argv: &[String], json: bool) -> Invocation {
    let parsed = crate::cli::parse(argv, FLAGS);
    let help = parsed.as_ref().is_ok_and(|p| p.bool_flag("help"));
    let help_text = self::help(group, parsed.as_ref().ok().and_then(|p| p.operand(0)));
    let input = (|| {
        let p = parsed?;
        if help {
            return Err("help requested".into());
        }
        if p.dash {
            return Err("arguments after -- are not accepted".into());
        }
        let accepted = allowed(group, p.operand(0));
        for flag in FLAGS {
            if p.occurrences(flag.name) > 0 && !accepted.contains(&flag.name) {
                return Err(format!("--{} is not valid for this operation", flag.name));
            }
        }
        if group == "request" {
            if !p.operands.is_empty()
                || p.flags.len() != p.occurrences("json") + p.occurrences("from")
            {
                return Err("request accepts --from and --json only".into());
            }
            return Ok(Input::File(p.value("from").unwrap_or("-").into()));
        }
        let leaf = p.operand(0).ok_or("missing operation")?;
        let host = p.operand(1).ok_or("missing host")?;
        let mut params = json!({});
        let operation = if group == "helper" {
            if p.operands.len() != 2 {
                return Err("helper accepts one host".into());
            }
            match leaf {
                "prepare" => "helper.prepare".to_owned(),
                "status" => "metadata".to_owned(),
                _ => return Err("unknown helper operation".into()),
            }
        } else {
            params["generation"] = json!(p.value("generation").ok_or("--generation is required")?);
            match leaf {
                "start" => {
                    if p.operands.len() != 2 {
                        return Err("run start accepts one host".into());
                    }
                    let key = p
                        .value("key")
                        .ok_or("--key is required before submission")?;
                    let expiry = super::request::key(key).map_err(|e| e.message)?;
                    let program = match (p.value("command"), p.value("argv-json")) {
                        (Some(script), None) => {
                            json!({"interpreter":p.value("shell").unwrap_or("/bin/sh"),"script":script,"login":p.bool_flag("login")})
                        }
                        (None, Some(argv)) => {
                            json!({"argv":serde_json::from_str::<Value>(argv).map_err(|e|e.to_string())?})
                        }
                        _ => {
                            return Err(
                                "exactly one of --command and --argv-json is required".into()
                            );
                        }
                    };
                    let mut env = BTreeMap::new();
                    for entry in p.all("env") {
                        let (name, value) =
                            entry.split_once('=').ok_or("--env requires KEY=VALUE")?;
                        if env.insert(name.to_owned(), value.to_owned()).is_some() {
                            return Err("duplicate environment name".into());
                        }
                    }
                    params["key"] = json!(key);
                    params["expires_unix"] = json!(expiry);
                    params["cwd"] = json!(p.value("cwd").unwrap_or("~"));
                    params["program"] = program;
                    params["environment"] =
                        json!({"inherit":p.bool_flag("inherit-env"),"values":env});
                    params["wait_ms"] = json!(milliseconds(p.value("wait"))?);
                    if p.value("deadline").is_some() {
                        params["deadline_ms"] = json!(milliseconds(p.value("deadline"))?);
                    }
                }
                "poll" | "signal" => {
                    if p.operands.len() != 3 {
                        return Err("run poll/signal requires host and run_id".into());
                    }
                    params["run_id"] = json!(p.operand(2));
                    if leaf == "signal" {
                        params["signal"] = json!(p.value("sig").ok_or("--sig is required")?);
                    } else {
                        params["wait_ms"] = json!(milliseconds(p.value("wait"))?);
                        for (flag, key, default) in [
                            ("stdout-since", "stdout_since", 0),
                            ("stderr-since", "stderr_since", 0),
                            ("max-bytes", "max_bytes", 4096),
                        ] {
                            let n = p
                                .value(flag)
                                .map(str::parse::<u64>)
                                .transpose()
                                .map_err(|_| format!("--{flag} requires an integer"))?
                                .unwrap_or(default);
                            params[key] = json!(n);
                        }
                    }
                }
                "list" | "prune" => {
                    if p.operands.len() != 2 {
                        return Err("run list/prune accepts one host".into());
                    }
                }
                _ => return Err("unknown run operation".into()),
            }
            format!("run.{leaf}")
        };
        let raw = json!({"schema_version":3,"operation":operation,"host":host,"params":params})
            .to_string();
        super::request::prepare(&operation, host, &params.to_string()).map_err(|e| e.message)?;
        Ok(Input::Document(raw))
    })();
    Invocation {
        input,
        json: json || (group == "request" && !help),
        help,
        help_text,
    }
}
