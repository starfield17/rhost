//! Capability probing. The probe is read-only and runs through the same
//! execution path as real work, so a successful probe also proves batch
//! authentication, a usable bash, `setsid`, and completion evidence all work.
use super::exec::{self, Request};
use crate::domain::{ExecOutcome, Execution};
use crate::transport::openssh::{Client, MasterStatus};
use std::collections::BTreeMap;
use std::time::Duration;

/// The capabilities `doctor` asks about, in the order a human reads them.
pub const CAPABILITIES: [&str; 11] = [
    "bash",
    "tmux",
    "setsid",
    "ps",
    "rsync",
    "sha256sum",
    "base64",
    "stty",
    "flock",
    "python3",
    "realpath",
];

/// A read-only POSIX probe that prints `key=value` lines. The keys are part of
/// the contract: `data.capabilities` and every field below is derived from them.
///
/// The state directory it reports is this major version's own, the same one the
/// exec wrapper writes pid files into and sessions live under, so a doctor run
/// answers about the state *this* binary owns (CONTRACT.md PERSIST-002).
fn probe_script() -> String {
    format!(
        r#"em() {{ printf '%s=%s\n' "$1" "$2"; }}
em os "$( (. /etc/os-release 2>/dev/null; printf '%s' "${{PRETTY_NAME:-unknown}}") )"
em kernel "$(uname -r 2>/dev/null)"
em arch "$(uname -m 2>/dev/null)"
em user "$(id -un 2>/dev/null)"
em home "$HOME"
ls="$(getent passwd "$(id -un)" 2>/dev/null | cut -d: -f7)"
[ -n "$ls" ] || ls="${{SHELL:-unknown}}"
em login_shell "$ls"
for c in bash tmux setsid ps rsync sha256sum base64 stty flock python3 realpath; do
  # `command -v` answers with the path this execution environment would use, so
  # a wrapper or a shadowed `bash` is reported as the one a real run resolves.
  p=$(command -v "$c" 2>/dev/null) || p=
  if [ -n "$p" ]; then em "have_$c" yes; else em "have_$c" no; fi
  em "path_$c" "$p"
done
rd="${{RHOST_REMOTE_STATE:-{state}}}"
if mkdir -p "$rd" 2>/dev/null && [ -w "$rd" ]; then em state_dir_writable yes; else em state_dir_writable no; fi
em state_dir "$rd"
if grep -qi microsoft /proc/version 2>/dev/null; then em wsl yes; else em wsl no; fi
"#,
        state = crate::transport::protocol::DEFAULT_REMOTE_STATE_DIR
    )
}

/// What a host reported about itself. `online` is the only claim that depends on
/// a completed probe; everything else stays empty when the probe did not run.
#[derive(Debug, Clone)]
pub struct Report {
    pub host: String,
    pub online: bool,
    pub os: String,
    pub kernel: String,
    pub arch: String,
    pub user: String,
    pub home: String,
    pub login_shell: String,
    pub state_dir: String,
    pub state_dir_writable: bool,
    pub wsl: bool,
    pub capabilities: BTreeMap<String, bool>,
    /// The path this execution environment resolves each capability to, for the
    /// same key set as `capabilities`; `None` means the probe did not find it.
    /// The value is Bash's own `command -v` answer, so it is the path a real run
    /// uses, wrappers and all — never this client's guess (AGENTS.md §1).
    pub capability_paths: BTreeMap<String, Option<String>>,
    pub connection: Connection,
    /// `None` means rhost could not tell whether the run reused a master; it is
    /// never reported as false.
    pub connection_reused: Option<bool>,
}

/// The subset of OpenSSH's control-protocol evidence a caller may see.
#[derive(Debug, Clone)]
pub struct Connection {
    pub master_status: MasterStatus,
    pub control_path: String,
    pub master_pid: Option<u32>,
    pub diagnostic: Option<String>,
    pub stopped: bool,
}

pub struct Options<'a> {
    pub host: &'a str,
    pub timeout: Duration,
    pub fresh: bool,
}

fn connection(client: &Client, host: &str) -> Connection {
    let status = client.connection_status(host);
    Connection {
        master_status: status.master_status,
        control_path: status.control_path,
        master_pid: status.master_pid,
        diagnostic: (!status.diagnostic.is_empty()).then_some(status.diagnostic),
        stopped: status.stopped,
    }
}

/// The result of a probe: what the host reported, plus the execution outcome
/// when the probe itself did not complete.
pub struct Probe {
    pub report: Report,
    /// `Some` means the probe never completed, so the report is the honest
    /// "nothing was learned" snapshot rather than a capability list.
    pub failure: Option<ExecOutcome>,
}

/// Probes a host without assuming anything about it.
pub fn probe(client: &Client, options: &Options<'_>) -> Result<Probe, exec::ExecError> {
    let before = connection(client, options.host);
    let reused = reuse_evidence(client, options, &before);
    let probe = probe_script();
    let request = Request {
        host: options.host,
        command: &probe,
        cwd: None,
        env: &[],
        timeout: Some(options.timeout),
        capture: Some(exec::DEFAULT_JSON_CAPTURE),
        fresh: options.fresh,
    };
    let mut report = Report {
        host: options.host.to_string(),
        online: false,
        os: String::new(),
        kernel: String::new(),
        arch: String::new(),
        user: String::new(),
        home: String::new(),
        login_shell: String::new(),
        state_dir: String::new(),
        state_dir_writable: false,
        wsl: false,
        capabilities: BTreeMap::new(),
        capability_paths: BTreeMap::new(),
        connection: before,
        connection_reused: reused,
    };
    let outcome = exec::execute_captured(client, &probe, &request, None)?;
    if outcome.failure().is_some() || !matches!(outcome.execution(), Execution::Completed(_)) {
        return Ok(Probe {
            report,
            failure: Some(outcome),
        });
    }
    let values = parse_kv(&outcome.output().stdout.content());
    let (capabilities, capability_paths) = capability_maps(&values);
    report.capabilities = capabilities;
    report.capability_paths = capability_paths;
    report.online = true;
    report.os = field(&values, "os");
    report.kernel = field(&values, "kernel");
    report.arch = field(&values, "arch");
    report.user = field(&values, "user");
    report.home = field(&values, "home");
    report.login_shell = field(&values, "login_shell");
    report.state_dir = field(&values, "state_dir");
    report.state_dir_writable = flag(&values, "state_dir_writable");
    report.wsl = flag(&values, "wsl");
    Ok(Probe {
        report,
        failure: None,
    })
}

fn field(values: &BTreeMap<String, String>, key: &str) -> String {
    values.get(key).cloned().unwrap_or_default()
}

fn flag(values: &BTreeMap<String, String>, key: &str) -> bool {
    values.get(key).is_some_and(|value| value == "yes")
}

/// Did this probe ride an existing master? Only answered when it can be
/// answered: a fresh connection never reuses shared state, and a master that was
/// absent before the probe cannot have been reused.
fn reuse_evidence(client: &Client, options: &Options<'_>, before: &Connection) -> Option<bool> {
    if options.fresh {
        return Some(false);
    }
    match before.master_status {
        MasterStatus::Absent => Some(false),
        MasterStatus::Unknown => None,
        MasterStatus::Alive => {
            let after = connection(client, options.host);
            match (before.master_pid, after.master_pid) {
                (Some(earlier), Some(later))
                    if after.master_status == MasterStatus::Alive && earlier == later =>
                {
                    Some(true)
                }
                _ => None,
            }
        }
    }
}

/// Builds the two capability maps from one probe transcript. Both come from the
/// same key set, so a capability and its path can never disagree about which
/// names exist: `have_<name> == yes` and a non-empty `path_<name>` describe the
/// same lookup, and a probe that reported neither is a missing capability with a
/// null path.
fn capability_maps(
    values: &BTreeMap<String, String>,
) -> (BTreeMap<String, bool>, BTreeMap<String, Option<String>>) {
    let mut capabilities = BTreeMap::new();
    let mut paths = BTreeMap::new();
    for name in CAPABILITIES {
        let found = values
            .get(&format!("have_{name}"))
            .is_some_and(|value| value == "yes");
        capabilities.insert(name.to_string(), found);
        let path = values
            .get(&format!("path_{name}"))
            .filter(|value| !value.is_empty())
            .cloned();
        paths.insert(name.to_string(), path);
    }
    (capabilities, paths)
}

/// Parses `key=value` lines, splitting on the first `=`.
fn parse_kv(text: &str) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    for line in text.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.is_empty() {
            continue;
        }
        values.insert(key.to_string(), value.to_string());
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_maps_name_the_same_keys_and_agree_on_what_was_found() {
        // A host where `bash` and `tmux` are present and `flock` is missing.
        let values = parse_kv(
            "have_bash=yes\npath_bash=/usr/bin/bash\n\
             have_tmux=yes\npath_tmux=/opt/bin/tmux\n\
             have_flock=no\npath_flock=\n",
        );
        let (capabilities, paths) = capability_maps(&values);
        assert_eq!(capabilities.len(), CAPABILITIES.len());
        assert_eq!(paths.len(), CAPABILITIES.len());
        assert_eq!(
            capabilities.keys().collect::<Vec<_>>(),
            paths.keys().collect::<Vec<_>>(),
            "the two maps must publish one key set"
        );
        assert_eq!(capabilities.get("bash"), Some(&true));
        assert_eq!(
            paths.get("bash").and_then(Option::as_deref),
            Some("/usr/bin/bash")
        );
        assert_eq!(capabilities.get("flock"), Some(&false));
        assert_eq!(
            paths.get("flock"),
            Some(&None),
            "a missing capability has no resolved path"
        );
        // A probe that reported nothing leaves every name missing with no path.
        let (empty, empty_paths) = capability_maps(&BTreeMap::new());
        assert_eq!(empty.values().filter(|found| **found).count(), 0);
        assert!(empty_paths.values().all(Option::is_none));
    }

    #[test]
    fn key_values_split_on_the_first_equals_only() {
        let values = parse_kv("a=1\r\n=skip\nb=x=y\n\nnoequals\n");
        assert_eq!(values.get("a").map(String::as_str), Some("1"));
        assert_eq!(values.get("b").map(String::as_str), Some("x=y"));
        assert_eq!(values.len(), 2);
    }

    #[test]
    fn the_probe_asks_about_every_published_capability() {
        let probe = probe_script();
        for name in CAPABILITIES {
            assert!(probe.contains(name), "probe no longer reports {name}");
        }
        // The path is resolved by the same `command -v` that answered, so a
        // capability and its path cannot come from two different lookups.
        assert!(probe.contains("path_$c"), "probe stopped resolving paths");
        // The state directory the probe reports is the one this version owns.
        assert!(probe.contains(crate::transport::protocol::DEFAULT_REMOTE_STATE_DIR));
    }
}
