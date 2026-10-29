//! CLI adaptation: load local input, execute once, then render the outcome.
use super::application::{self, Outcome, Request};
use super::command::{Session, budget};
use super::{dto, render};
use crate::cli::{Failure, Sink, warn};
use crate::transport::Client;

pub fn run(sink: &mut Sink, client: &Client, command: Session, json: bool) -> u8 {
    let (host, request) = match command {
        Session::Create {
            host,
            name,
            cwd,
            shell,
            timeout_nanos,
        } => (
            host,
            Request::Create {
                name,
                cwd,
                shell: if shell.is_empty() {
                    super::DEFAULT_SHELL.into()
                } else {
                    shell
                },
                timeout: budget(timeout_nanos),
            },
        ),
        Session::List {
            host,
            timeout_nanos,
        } => (
            host,
            Request::List {
                timeout: budget(timeout_nanos),
            },
        ),
        Session::Exec {
            host,
            session,
            program,
            timeout_nanos,
        } => {
            let command = match crate::cli::read_command_file(&program) {
                Ok(command) => command,
                Err(reason) => {
                    return Failure::new("session.exec", &host, "CONFIG_INVALID", reason)
                        .deliver(sink, json);
                }
            };
            (
                host,
                Request::Exec {
                    session,
                    command,
                    timeout: budget(timeout_nanos),
                },
            )
        }
        Session::Send {
            host,
            session,
            data,
            key,
            enter,
        } => (
            host,
            Request::Send {
                session,
                data,
                key,
                enter,
            },
        ),
        Session::Read {
            host,
            session,
            offset,
            max_bytes,
            timeout_nanos,
        } => (
            host,
            Request::Read {
                session,
                offset,
                max_bytes,
                timeout: budget(timeout_nanos),
            },
        ),
        Session::Recover {
            host,
            session,
            timeout_nanos,
        } => (
            host,
            Request::Recover {
                session,
                timeout: budget(timeout_nanos),
            },
        ),
        Session::Close { host, session } => (host, Request::Close { session }),
    };
    let outcome = match application::execute(client, &host, &request, &|| false) {
        Ok(outcome) => outcome,
        Err(error) => {
            return Failure::from_error(request.operation(), &host, error).deliver(sink, json);
        }
    };
    let status = status(&outcome, &host);
    if json {
        match outcome.envelope(&host) {
            Ok(value) => sink.line(&value.to_string()),
            Err(error) => {
                warn(&format!("rhost: INTERNAL: {error}"));
                return 255;
            }
        }
    } else {
        human(sink, &outcome);
    }
    status
}
fn status(outcome: &Outcome, host: &str) -> u8 {
    match outcome {
        Outcome::Created(value) => match &value.error {
            Some(error) => Failure::from_error("session.create", host, error.clone()).status(),
            None => 0,
        },
        Outcome::Executed(value) => dto::session_exec_status(value),
        Outcome::Recovered(value) => match &value.failure {
            Some(error) => Failure::from_error("session.recover", host, error.clone()).status(),
            None => 0,
        },
        _ => 0,
    }
}
fn human(sink: &mut Sink, outcome: &Outcome) {
    match outcome {
        Outcome::Created(value) => match &value.info {
            Some(info) => render::session_created(sink, info),
            None => {
                let (code, message) = value
                    .error
                    .as_ref()
                    .map(|e| (e.code, e.message.as_str()))
                    .unwrap_or(("INTERNAL", "missing create failure"));
                warn(&format!(
                    "rhost: {code}: {message} (candidate session {}; run `rhost session list` to check it)",
                    value.candidate_id
                ));
            }
        },
        Outcome::Listed(rows) => render::session_list(sink, rows),
        Outcome::Executed(value) => match dto::session_exec_failure(value) {
            Some((code, message, _)) => {
                warn(&format!("rhost: {code}: {message}"));
                if code == "REMOTE_COMMAND_TIMEOUT" {
                    warn(&format!(
                        "  session is {}",
                        if value.preserved() {
                            "usable again"
                        } else {
                            "still busy"
                        }
                    ));
                }
            }
            None => render::session_output(
                sink,
                value.output().0.content().as_ref(),
                value.output().0.truncated(),
            ),
        },
        Outcome::Sent => sink.line("sent"),
        Outcome::Read(value) => render::session_output(sink, &value.content, false),
        Outcome::Recovered(value) => match &value.failure {
            Some(error) => warn(&format!("rhost: {}: {}", error.code, error.message)),
            None => sink.line("session is responsive"),
        },
        Outcome::Closed => sink.line("closed"),
    }
}
