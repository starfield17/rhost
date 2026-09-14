//! Tunnels: one forward, one dedicated OpenSSH master, one local record.
//!
//! The stub stands in for OpenSSH itself, so it answers the control protocol and
//! creates or removes the socket file its `ControlPath` names. That is what makes
//! `alive` checkable here: a record that disagrees with the socket is worse than
//! no record, because the caller keeps an id that names nothing.

mod open;
mod records;

use crate::support::{Harness, envelope, fail, operation, run, want_code};
use regex::Regex;

/// A master that keeps its own socket. The mode is read from a file rather than
/// the environment because a case has to change it *between* two CLI processes:
/// one opens a forward, the next one asks whether it is still there.
const MASTER_SCRIPT: &str = r#"mode=present
if [ -f "$RHOST_TEST_TUNNEL_MODE_FILE" ]; then
  mode=$(cat "$RHOST_TEST_TUNNEL_MODE_FILE")
fi
socket=""
previous=""
for argument in "$@"; do
  if [ "$previous" = "-S" ] && [ -z "$socket" ]; then
    socket="$argument"
  fi
  case "$argument" in
    # The first ControlPath wins, exactly as OpenSSH treats the repeated option.
    ControlPath=*) if [ -z "$socket" ]; then socket=${argument#ControlPath=}; fi ;;
  esac
  previous="$argument"
done
case "$*" in
  *"-O check"*)
    if [ "$mode" = "uncertain" ]; then
      echo "Control socket connect($socket): Permission denied" >&2
      exit 255
    fi
    if [ -n "$socket" ] && [ -e "$socket" ]; then
      echo "Master running (pid=4321)" >&2
      exit 0
    fi
    echo "Control socket connect($socket): No such file or directory" >&2
    exit 255
    ;;
  *"-O exit"*)
    if [ -n "$socket" ]; then
      rm -f "$socket"
    fi
    echo "Exit request sent." >&2
    exit 0
    ;;
esac
case "$*" in
  *"-fNT"*)
    if [ "$mode" = "refused" ]; then
      echo "Warning: remote port forwarding failed for listen port 9000" >&2
      exit 255
    fi
    if [ -n "$socket" ] && [ "$mode" != "silent" ]; then
      : > "$socket"
    fi
    exit 0
    ;;
esac
exit 2
"#;

/// A harness whose `ssh` is that master, with `mode` as its first answer.
fn tunnel_master(case: &str, mode: &str) -> Result<Harness, String> {
    let harness = Harness::open(case)?;
    harness.stub("ssh", MASTER_SCRIPT)?;
    harness.write("tunnel-mode", mode, None)?;
    let mode_file = harness.path("tunnel-mode").to_string_lossy().to_string();
    // The audit trail is its own subsystem with its own cases; here the state
    // root has to hold records and nothing else, so nothing else is written.
    Ok(harness
        .env("RHOST_TEST_TUNNEL_MODE_FILE", &mode_file)
        .env("RHOST_AUDIT", "0"))
}

/// Every local file this candidate owns under its state root. Tunnels are the
/// only thing that writes there in these cases, so "no record" is an assertion
/// about what a later process can rediscover rather than about a path.
fn state_files(harness: &Harness) -> Result<Vec<String>, String> {
    let root = harness.path("state");
    let mut found: Vec<String> = Vec::new();
    let mut pending = vec![root];
    while let Some(directory) = pending.pop() {
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                found.push(path.to_string_lossy().into_owned());
            }
        }
    }
    found.sort();
    Ok(found)
}

fn opened_id(value: &serde_json::Value) -> Result<String, String> {
    value
        .pointer("/data/tunnel_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("no tunnel_id in {value}"))
}

fn open_call(harness: &Harness) -> Result<Vec<String>, String> {
    harness
        .tool_calls("ssh")
        .into_iter()
        .find(|call| call.args.iter().any(|argument| argument == "-fNT"))
        .map(|call| call.args)
        .ok_or_else(|| format!("no master-bearing ssh ran:\n{}", harness.calls_text()))
}

/// The socket the master was told to own: the first `ControlPath`, which is the
/// one OpenSSH keeps.
fn master_socket(harness: &Harness) -> Result<String, String> {
    open_call(harness)?
        .iter()
        .find_map(|argument| argument.strip_prefix("ControlPath="))
        .map(str::to_string)
        .ok_or_else(|| "the master was started without a ControlPath".to_string())
}
