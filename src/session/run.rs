//! Running one session operation: the envelope or the human view, plus the
//! process status the answer implies.

use super::command::{CLOSE_TIMEOUT, SEND_TIMEOUT, Session, budget};
use super::{dto, ops as app_session, render};
use crate::audit::Timer;
use crate::cli::{Failure, Sink, warn};
use crate::remote::Error as AppError;
use crate::transport::Client;

/// Runs one session operation: the envelope or the human view, plus the process
/// status the answer implies.
pub fn run(sink: &mut Sink, client: &Client, command: Session, json: bool) -> u8 {
    match command {
        Session::Create {
            host,
            name,
            cwd,
            shell,
            timeout_nanos,
        } => {
            let timer = Timer::start("session.create", &host);
            let options = app_session::CreateOptions {
                host: &host,
                name: &name,
                cwd: &cwd,
                shell: &shell,
                timeout: budget(timeout_nanos),
            };
            match app_session::create(client, &options) {
                Ok(created) => match &created.info {
                    Some(info) => {
                        timer.succeeded(&info.initial_cwd, "", None);
                        if json {
                            sink.envelope(&dto::session_created(&host, &created));
                        } else {
                            render::session_created(sink, info);
                        }
                        0
                    }
                    // The request reached the host, so the answer names the
                    // candidate identity and what became of it, on both paths.
                    None => {
                        let error = created
                            .error
                            .clone()
                            .unwrap_or_else(|| AppError::new("INTERNAL", "missing create failure"));
                        timer.failed(error.code);
                        if json {
                            sink.envelope(&dto::session_create_failure(&host, &created));
                        } else {
                            warn(&format!(
                                "rhost: {}: {}",
                                error.code,
                                created_failure_message(&created)
                            ));
                        }
                        Failure::from_error("session.create", &host, error).status()
                    }
                },
                Err(error) => {
                    timer.failed(error.code);
                    Failure::from_error("session.create", &host, error).deliver(sink, json)
                }
            }
        }
        Session::List {
            host,
            timeout_nanos,
        } => match app_session::list(client, &host, budget(timeout_nanos)) {
            Ok(rows) => {
                if json {
                    sink.envelope(&dto::sessions(&host, &rows));
                } else {
                    render::session_list(sink, &rows);
                }
                0
            }
            Err(error) => Failure::from_error("session.list", &host, error).deliver(sink, json),
        },
        Session::Exec {
            host,
            session,
            command,
            timeout_nanos,
        } => {
            let timer = Timer::start("session.exec", &host);
            let options = app_session::ExecOptions {
                host: &host,
                session: &session,
                command: &command,
                timeout: budget(timeout_nanos),
            };
            match app_session::exec(client, &options) {
                Ok(outcome) => {
                    let status = dto::session_exec_status(&outcome);
                    match dto::session_exec_failure(&outcome) {
                        Some((code, _, _)) => timer.failed(code),
                        None => {
                            if let crate::domain::Execution::Completed(code) = outcome.execution() {
                                timer.succeeded("", &command, Some(code.get()));
                            }
                        }
                    }
                    if json {
                        sink.envelope(&dto::session_exec(&host, &outcome));
                    } else if let Some((code, message, _)) = dto::session_exec_failure(&outcome) {
                        warn(&format!("rhost: {code}: {message}"));
                        // Only a timeout leaves the question "is the pane free
                        // again?"; a refusal like SESSION_BUSY is not a damaged
                        // session, it is a busy one.
                        if code == "REMOTE_COMMAND_TIMEOUT" {
                            warn(&format!(
                                "  session is {}",
                                if outcome.preserved() {
                                    "usable again"
                                } else {
                                    "still busy"
                                }
                            ));
                        }
                    } else {
                        render::session_output(sink, outcome.output().0.content().as_ref());
                    }
                    status
                }
                Err(error) => {
                    timer.failed(error.code);
                    Failure::from_error("session.exec", &host, error).deliver(sink, json)
                }
            }
        }
        Session::Send {
            host,
            session,
            data,
            key,
            enter,
        } => {
            let timer = Timer::start("session.send", &host);
            let options = app_session::SendOptions {
                host: &host,
                session: &session,
                data: data.as_deref(),
                key: key.as_deref(),
                enter,
            };
            match app_session::send(client, &options, SEND_TIMEOUT) {
                Ok(()) => {
                    // The injected text may be sensitive, so the trail records the
                    // key it pressed and never what it pasted.
                    let summary = match (&key, &data) {
                        (Some(key), _) => format!("key {key}"),
                        _ => "--data (redacted)".to_string(),
                    };
                    timer.succeeded("", &summary, None);
                    if json {
                        sink.envelope(&dto::session_sent(&host));
                    } else {
                        sink.line("sent");
                    }
                    0
                }
                Err(error) => {
                    timer.failed(error.code);
                    Failure::from_error("session.send", &host, error).deliver(sink, json)
                }
            }
        }
        Session::Read {
            host,
            session,
            since,
            timeout_nanos,
        } => match app_session::read(client, &host, &session, since, budget(timeout_nanos)) {
            Ok(result) => {
                if json {
                    sink.envelope(&dto::session_read(&host, &result));
                } else {
                    render::session_output(sink, &result.content);
                }
                0
            }
            Err(error) => Failure::from_error("session.read", &host, error).deliver(sink, json),
        },
        Session::Recover {
            host,
            session,
            timeout_nanos,
        } => {
            let timer = Timer::start("session.recover", &host);
            match app_session::recover(client, &host, &session, budget(timeout_nanos)) {
                Ok(result) => {
                    let status = match &result.failure {
                        None => 0,
                        Some(error) => Failure::new(
                            "session.recover",
                            &host,
                            error.code,
                            error.message.clone(),
                        )
                        .status(),
                    };
                    match &result.failure {
                        None => timer.succeeded("", "key C-c and responsiveness probe", None),
                        Some(error) => timer.failed(error.code),
                    }
                    if json {
                        sink.envelope(&dto::session_recover(&host, &result));
                    } else if let Some(error) = &result.failure {
                        warn(&format!("rhost: {}: {}", error.code, error.message));
                    } else {
                        sink.line("session is responsive");
                    }
                    status
                }
                Err(error) => {
                    timer.failed(error.code);
                    Failure::from_error("session.recover", &host, error).deliver(sink, json)
                }
            }
        }
        Session::Close { host, session } => {
            let timer = Timer::start("session.close", &host);
            match app_session::close(client, &host, &session, CLOSE_TIMEOUT) {
                Ok(()) => {
                    timer.succeeded("", &session, None);
                    if json {
                        sink.envelope(&dto::session_closed(&host));
                    } else {
                        sink.line("closed");
                    }
                    0
                }
                Err(error) => {
                    timer.failed(error.code);
                    Failure::from_error("session.close", &host, error).deliver(sink, json)
                }
            }
        }
    }
}

/// The human text for a create that did not return a full record. It names the
/// candidate identity and points at the one way to find out what happened: list
/// the sessions.
fn created_failure_message(created: &app_session::Created) -> String {
    let message = created
        .error
        .as_ref()
        .map(|error| error.message.clone())
        .unwrap_or_else(|| "session creation did not return a record".to_string());
    format!(
        "{message} (candidate session {}; run `rhost session list` to check it)",
        created.candidate_id
    )
}
