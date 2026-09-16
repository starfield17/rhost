//! `create`: make one remote tmux session and return the record for it.
//!
//! Creation is the one session operation that can leave state behind when its
//! answer is lost: the remote creates the tmux session before it prints its
//! completion line. So this use-case reserves the candidate identity *before*
//! submitting, and every failure reports that candidate plus what is actually
//! known about it, rather than pretending nothing happened (SESSION-010).

use super::errors::{run_helper_once, session_error, unhealthy, valid_name};
use super::lifecycle::Info;
use super::shared::{OUTPUT_CAPTURE, now};
use crate::app::Error;
use crate::app::remote::{RemoteRun, internal};
use crate::session::{self, Meta, Status};
use crate::transport::Client;
use std::time::Duration;

/// What a caller passes to `session create`.
pub struct CreateOptions<'a> {
    pub host: &'a str,
    /// Empty means "use the id as the name".
    pub name: &'a str,
    pub cwd: &'a str,
    pub shell: &'a str,
    pub timeout: Duration,
}

/// What is known about a session after a create that did not return its full
/// record. The three values are exhaustive: the remote refused (or was seen to
/// clean up), the remote was seen to create it but the answer was incomplete, or
/// nothing conclusive was observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreationStatus {
    /// The remote explicitly refused, or was observed to clean up. No session
    /// with the candidate identity remains.
    NotCreated,
    /// A timeout, cancellation, disconnect, or missing completion evidence. A
    /// session may exist; it is not proven either way.
    Unknown,
    /// Creation evidence was observed, but the rest of the response was not.
    /// The session is expected to exist.
    Created,
}

impl CreationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotCreated => "not_created",
            Self::Unknown => "unknown",
            Self::Created => "created",
        }
    }
}

/// The answer to `session create`. A success carries the complete [`Info`]; any
/// other outcome carries the candidate identity and what was observed, so a
/// caller can look the session up instead of retrying blind.
pub struct Created {
    pub candidate_id: String,
    pub candidate_ref: String,
    pub creation_status: CreationStatus,
    /// Present only when creation was proven and the record is complete.
    pub info: Option<Info>,
    /// Present only when `info` is `None`.
    pub error: Option<Error>,
}

impl Created {
    pub fn succeeded(&self) -> bool {
        self.info.is_some()
    }

    /// A create that reached the host but did not return its full record, for
    /// callers that describe the failure shape rather than run one.
    pub fn failed(
        candidate_id: &str,
        candidate_ref: &str,
        creation_status: CreationStatus,
        code: &'static str,
        message: &str,
        retryable: bool,
    ) -> Self {
        Self {
            candidate_id: candidate_id.to_string(),
            candidate_ref: candidate_ref.to_string(),
            creation_status,
            info: None,
            error: Some(Error {
                code,
                message: message.to_string(),
                retryable,
            }),
        }
    }
}

/// Creates a session and returns the record the remote host now owns.
///
/// A local mistake (bad shell, bad name, an id that cannot be generated) is
/// `Err` with no session to report. Once the request has been submitted, the
/// outcome is always `Ok`: the caller learns the candidate identity and whether
/// the remote created, refused, or left the answer unknown.
pub fn create(client: &Client, options: &CreateOptions<'_>) -> Result<Created, Error> {
    let shell_name = if options.shell.is_empty() {
        session::DEFAULT_SHELL
    } else {
        options.shell
    };
    if shell_name != session::DEFAULT_SHELL {
        return Err(Error::new(
            "CONFIG_INVALID",
            "only --shell bash is supported",
        ));
    }
    if !options.name.is_empty() && !valid_name(options.name) {
        return Err(Error::new(
            "CONFIG_INVALID",
            "invalid session name (use letters, digits, _ . -)",
        ));
    }
    let id = session::new_id()
        .map_err(|error| internal(format!("could not generate session id: {error}")))?;
    let name = if options.name.is_empty() {
        id.clone()
    } else {
        options.name.to_string()
    };
    let meta = Meta::new(&id, &name, options.cwd, shell_name, &now());
    let script = session::create_script(&meta, "bash --noprofile --norc -i");
    let run = run_helper_once(
        client,
        options.host,
        &script,
        options.timeout,
        OUTPUT_CAPTURE,
    )?;
    // A failure before the command was submitted (bad target, authentication,
    // host key, unreachable host, missing remote dependency) proves nothing was
    // created, so there is no candidate session to report and this stays the
    // ordinary null-data failure. Once the request may have reached the host,
    // the candidate travels with the answer instead.
    if let Some(failure) = &run.failure {
        if !reached_host(failure.code) {
            return Err(failure.clone());
        }
    }
    Ok(classify(&meta, options.cwd, &run))
}

/// Whether an error code can follow a submission that reached the host. Codes
/// that are decided before the command crosses the SSH boundary mean no session
/// was made; everything else leaves the outcome genuinely open.
fn reached_host(code: &str) -> bool {
    matches!(
        code,
        "REMOTE_COMMAND_TIMEOUT"
            | "REMOTE_COMMAND_CANCELLED"
            | "OUTPUT_WRITE_FAILED"
            | "REMOTE_EXECUTION_UNKNOWN"
    )
}

/// What one submitted create amounts to. Pure so every branch — and every
/// creation status — can be decided from a transcript without a host.
fn classify(meta: &Meta, requested_cwd: &str, run: &RemoteRun) -> Created {
    let id = meta.id.clone();
    let name = meta.name.clone();
    let stdout = run.stdout.as_str();
    let created_evidence = session::field(stdout, "RHOST_CREATED=").filter(|value| *value == id);
    let refusal = session::field(stdout, "RHOST_ERR=");

    // An explicit refusal is the remote's own word that it did not keep a
    // session: every refusal path either never created one or cleaned it up
    // under its EXIT trap, so it is reported as a definite negative.
    if let Some(code) = refusal {
        let error = session_error(session::HelperFailure::from_code(code));
        return Created {
            candidate_id: id,
            candidate_ref: name,
            creation_status: CreationStatus::NotCreated,
            info: None,
            error: Some(error),
        };
    }

    // Success needs the whole record, not merely a session: the completion line
    // is only printed once `meta.json` exists.
    if stdout.contains("RHOST_OK=created") && run.status == Some(0) {
        let info = Info {
            id: id.clone(),
            name: name.clone(),
            tmux_session: meta.tmux_session.clone(),
            created_at: meta.created_at.clone(),
            initial_cwd: resolve_reported_cwd(stdout, requested_cwd),
            shell: meta.shell.clone(),
            status: Status::Alive,
        };
        return Created {
            candidate_id: id,
            candidate_ref: name,
            creation_status: CreationStatus::Created,
            info: Some(info),
            error: None,
        };
    }

    // A transport failure keeps its own code and retryability: a timeout is
    // still `REMOTE_COMMAND_TIMEOUT`, not a session-specific rewrite. Only the
    // *creation status* is refined by what was observed.
    if let Some(error) = &run.failure {
        let status = if created_evidence.is_some() {
            CreationStatus::Created
        } else {
            CreationStatus::Unknown
        };
        return Created {
            candidate_id: id,
            candidate_ref: name,
            creation_status: status,
            info: None,
            error: Some(error.clone()),
        };
    }

    // Creation evidence without a complete record and without a transport
    // failure: the session is expected to exist, so the caller should list
    // rather than assume it does not.
    if created_evidence.is_some() {
        return Created {
            candidate_id: id,
            candidate_ref: name,
            creation_status: CreationStatus::Created,
            info: None,
            error: Some(unhealthy(
                "session was created but its record could not be confirmed; run `rhost session list`",
            )),
        };
    }

    // The helper completed but did not report a known failure or success. That
    // is the same session-level unhealth as before this change, not a new code.
    if let Some(code) = run.status {
        return Created {
            candidate_id: id,
            candidate_ref: name,
            creation_status: CreationStatus::Unknown,
            info: None,
            error: Some(unhealthy(&format!("session helper exited {code}"))),
        };
    }

    // No refusal, no creation evidence, no completion: the request may or may
    // not have taken effect.
    Created {
        candidate_id: id,
        candidate_ref: name,
        creation_status: CreationStatus::Unknown,
        info: None,
        error: Some(Error::new(
            "REMOTE_EXECUTION_UNKNOWN",
            "session creation completion evidence is missing",
        )),
    }
}

/// The absolute directory the remote reported for `--cwd`, or the caller's own
/// literal when none was requested. A requested cwd always resolves, so the
/// literal only stands in on the impossible success-without-report path.
fn resolve_reported_cwd(stdout: &str, requested: &str) -> String {
    match session::field(stdout, "RHOST_CWD=").filter(|value| !value.is_empty()) {
        Some(resolved) => resolved.to_string(),
        None if requested.is_empty() => String::new(),
        None => requested.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(cwd: &str) -> Meta {
        Meta::new("s_candidate", "work", cwd, session::DEFAULT_SHELL, "unix:1")
    }

    fn run(stdout: &str, status: Option<u8>) -> RemoteRun {
        RemoteRun {
            stdout: stdout.to_string(),
            stderr: String::new(),
            status,
            failure: None,
        }
    }

    #[test]
    fn a_full_record_is_created_and_reports_the_resolved_cwd() {
        let outcome = classify(
            &meta("~"),
            "~",
            &run(
                "RHOST_CWD=/home/dev\nRHOST_CREATED=s_candidate\nRHOST_OK=created\n",
                Some(0),
            ),
        );
        assert!(outcome.succeeded());
        assert_eq!(outcome.creation_status, CreationStatus::Created);
        let info = outcome
            .info
            .unwrap_or_else(|| panic!("create succeeded without a record"));
        assert_eq!(
            info.initial_cwd, "/home/dev",
            "the resolved path, not the literal `~`"
        );
        assert_eq!(info.id, "s_candidate");
    }

    #[test]
    fn creation_evidence_without_a_record_is_created_not_unknown() {
        // The remote printed its created line, then the answer was cut off.
        let outcome = classify(&meta("~"), "~", &run("RHOST_CREATED=s_candidate\n", None));
        assert!(!outcome.succeeded());
        assert_eq!(
            outcome.creation_status,
            CreationStatus::Created,
            "observed creation evidence is not the same as an unknown outcome"
        );
        assert_eq!(
            outcome.error.as_ref().map(|error| error.code),
            Some("SESSION_UNHEALTHY")
        );
    }

    #[test]
    fn a_refusal_is_a_definite_negative() {
        let outcome = classify(&meta("~"), "~", &run("RHOST_ERR=nameinuse\n", Some(0)));
        assert_eq!(outcome.creation_status, CreationStatus::NotCreated);
        assert_eq!(
            outcome.error.as_ref().map(|error| error.code),
            Some("CONFIG_INVALID")
        );
    }

    #[test]
    fn creation_evidence_with_a_transport_failure_keeps_the_transport_code() {
        // The remote printed its created line, then the channel timed out: the
        // status is `created`, but the code and retryability stay the timeout's,
        // never a session-specific rewrite.
        let mut timed_out = run("RHOST_CREATED=s_candidate\n", None);
        timed_out.failure = Some(Error::new("REMOTE_COMMAND_TIMEOUT", "deadline"));
        let outcome = classify(&meta("~"), "~", &timed_out);
        assert_eq!(outcome.creation_status, CreationStatus::Created);
        assert_eq!(
            outcome.error.as_ref().map(|error| error.code),
            Some("REMOTE_COMMAND_TIMEOUT"),
            "a timeout must keep its own code even with creation evidence"
        );
        assert!(
            !outcome.error.as_ref().is_some_and(|error| error.retryable),
            "a timeout that may have created a session stays non-retryable"
        );
    }

    #[test]
    fn silence_is_unknown_and_keeps_the_transport_cause() {
        // No refusal, no creation evidence, no completion: the request may or
        // may not have taken effect.
        let mut silent = run("", None);
        silent.failure = Some(Error::new("REMOTE_COMMAND_TIMEOUT", "deadline"));
        let outcome = classify(&meta("~"), "~", &silent);
        assert_eq!(outcome.creation_status, CreationStatus::Unknown);
        assert_eq!(
            outcome.error.as_ref().map(|error| error.code),
            Some("REMOTE_COMMAND_TIMEOUT"),
            "the transport cause is preserved, not overwritten"
        );

        // A helper that completed without printing its record is the same
        // session-unhealthy answer as before this change, not a new code.
        let bare = classify(&meta("~"), "~", &run("", Some(0)));
        assert_eq!(bare.creation_status, CreationStatus::Unknown);
        assert_eq!(
            bare.error.as_ref().map(|error| error.code),
            Some("SESSION_UNHEALTHY")
        );
        // A run with neither a status nor a transport failure is execution
        // uncertainty, which is the exec layer's own code.
        let incomplete = classify(&meta("~"), "~", &run("", None));
        assert_eq!(
            incomplete.error.as_ref().map(|error| error.code),
            Some("REMOTE_EXECUTION_UNKNOWN")
        );
    }

    #[test]
    fn a_create_without_a_cwd_reports_none() {
        let outcome = classify(&meta(""), "", &run("RHOST_OK=created\n", Some(0)));
        let info = outcome
            .info
            .unwrap_or_else(|| panic!("create succeeded without a record"));
        assert_eq!(info.initial_cwd, "");
    }
}
