//! Shared session operations, complete outcomes and one audit decision per call.
use super::{dto, ops};
use crate::audit::Timer;
use crate::remote::{Error, Observation};
use crate::transport::Client;
use serde_json::Value;
use std::time::Duration;
pub(super) const CREATE_TIMEOUT: Duration = Duration::from_secs(60);
pub(super) const LIST_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const EXEC_TIMEOUT: Duration = Duration::from_secs(60);
pub(super) const SEND_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const READ_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const RECOVER_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const CLOSE_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) enum Request {
    Create {
        name: String,
        cwd: String,
        shell: String,
        timeout: Duration,
    },
    List {
        timeout: Duration,
    },
    Exec {
        session: String,
        command: String,
        timeout: Duration,
    },
    Send {
        session: String,
        data: Option<String>,
        key: Option<String>,
        enter: bool,
    },
    Read {
        session: String,
        offset: ops::ReadOffset,
        max_bytes: usize,
        timeout: Duration,
    },
    Recover {
        session: String,
        timeout: Duration,
    },
    Close {
        session: String,
    },
}
pub(crate) enum Outcome {
    Created(ops::Created),
    Listed(Vec<ops::Info>),
    Executed(crate::domain::SessionExecOutcome),
    Sent,
    Read(ops::Read),
    Recovered(ops::Recover),
    Closed,
}
impl Request {
    pub fn validate(&self) -> Result<(), Error> {
        let invalid = |message| Error::new("CONFIG_INVALID", message);
        let reference = match self {
            Self::Exec { session, .. }
            | Self::Send { session, .. }
            | Self::Read { session, .. }
            | Self::Recover { session, .. }
            | Self::Close { session } => Some(session),
            _ => None,
        };
        if let Some(reference) = reference {
            crate::domain::SessionRef::new(reference.clone())
                .map_err(|e| invalid(e.to_string()))?;
        }
        match self {
            Self::Create {
                name, cwd, shell, ..
            } => {
                if !name.is_empty() && !ops::valid_name(name) {
                    return Err(invalid("invalid session name".into()));
                }
                if shell != "bash" || cwd.contains('\0') {
                    return Err(invalid("session requires bash and a valid cwd".into()));
                }
            }
            Self::Exec { command, .. } => {
                if command.trim().is_empty() || command.contains('\0') {
                    return Err(invalid("no valid command given".into()));
                }
            }
            Self::Read { max_bytes, .. } => {
                if !(4..=super::MAX_READ_BYTES).contains(max_bytes) {
                    return Err(invalid("invalid read budget".into()));
                }
            }
            Self::Send {
                data, key, enter, ..
            } => {
                let data = data.as_deref().filter(|s| !s.is_empty());
                let key = key.as_deref().filter(|s| !s.is_empty());
                match (data, key) {
                    (Some(data), None) if !data.contains('\0') => {}
                    (None, Some(key)) if !*enter && ops::is_supported_key(key) => {}
                    _ => {
                        return Err(invalid(
                            "provide data with optional enter, or one supported key".into(),
                        ));
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub fn operation(&self) -> &'static str {
        match self {
            Self::Create { .. } => "session.create",
            Self::List { .. } => "session.list",
            Self::Exec { .. } => "session.exec",
            Self::Send { .. } => "session.send",
            Self::Read { .. } => "session.read",
            Self::Recover { .. } => "session.recover",
            Self::Close { .. } => "session.close",
        }
    }
}
impl Outcome {
    pub fn envelope(&self, host: &str) -> Result<Value, serde_json::Error> {
        match self {
            Self::Created(value) => match &value.info {
                Some(info) => serde_json::to_value(dto::session_created(host, info)),
                None => serde_json::to_value(dto::session_create_failure(host, value)),
            },
            Self::Listed(value) => serde_json::to_value(dto::sessions(host, value)),
            Self::Executed(value) => serde_json::to_value(dto::session_exec(host, value)),
            Self::Sent => serde_json::to_value(dto::session_sent(host)),
            Self::Read(value) => serde_json::to_value(dto::session_read(host, value)),
            Self::Recovered(value) => serde_json::to_value(dto::session_recover(host, value)),
            Self::Closed => serde_json::to_value(dto::session_closed(host)),
        }
    }
}
pub(crate) fn execute(
    client: &Client,
    host: &str,
    request: &Request,
    cancelled: &dyn Fn() -> bool,
) -> Result<Outcome, Error> {
    let observation = Observation::new(client, cancelled);
    observation.check()?;
    let operation = request.operation();
    // Reads and discovery retain their existing absence from the mutation trail.
    let timer = if matches!(request, Request::List { .. } | Request::Read { .. }) {
        None
    } else {
        Some(Timer::start(operation, host))
    };
    let result = request.validate().and_then(|()| match request {
        Request::Create {
            name,
            cwd,
            shell,
            timeout,
        } => ops::create_observed(
            &observation,
            &ops::CreateOptions {
                host,
                name,
                cwd,
                shell,
                timeout: *timeout,
            },
        )
        .map(Outcome::Created),
        Request::List { timeout } => {
            ops::list_observed(&observation, host, *timeout).map(Outcome::Listed)
        }
        Request::Exec {
            session,
            command,
            timeout,
        } => ops::exec_observed(
            &observation,
            &ops::ExecOptions {
                host,
                session,
                command,
                timeout: *timeout,
            },
        )
        .map(Outcome::Executed),
        Request::Send {
            session,
            data,
            key,
            enter,
        } => ops::send_observed(
            &observation,
            &ops::SendOptions {
                host,
                session,
                data: data.as_deref(),
                key: key.as_deref(),
                enter: *enter,
            },
            SEND_TIMEOUT,
        )
        .map(|()| Outcome::Sent),
        Request::Read {
            session,
            offset,
            max_bytes,
            timeout,
        } => ops::read_observed(&observation, host, session, *offset, *max_bytes, *timeout)
            .map(Outcome::Read),
        Request::Recover { session, timeout } => {
            ops::recover_observed(&observation, host, session, *timeout).map(Outcome::Recovered)
        }
        Request::Close { session } => {
            ops::close_observed(&observation, host, session, CLOSE_TIMEOUT)
                .map(|()| Outcome::Closed)
        }
    });
    if let Some(timer) = timer {
        record(&timer, request, &result);
    }
    result
}
fn record(timer: &Timer, request: &Request, result: &Result<Outcome, Error>) {
    match result {
        Err(error) => timer.failed(error.code),
        Ok(Outcome::Created(created)) => match &created.info {
            Some(info) => timer.succeeded(&info.initial_cwd, "", None),
            None => timer.failed(created.error.as_ref().map(|e| e.code).unwrap_or("INTERNAL")),
        },
        Ok(Outcome::Executed(value)) => match dto::session_exec_failure(value) {
            Some((code, _, _)) => timer.failed(code),
            None => {
                if let (Request::Exec { command, .. }, crate::domain::Execution::Completed(code)) =
                    (request, value.execution())
                {
                    timer.succeeded("", command, Some(code.get()));
                }
            }
        },
        Ok(Outcome::Recovered(value)) => match &value.failure {
            Some(error) => timer.failed(error.code),
            None => timer.succeeded("", "key C-c and responsiveness probe", None),
        },
        Ok(Outcome::Sent) => {
            let summary = match request {
                Request::Send { key: Some(key), .. } => format!("key {key}"),
                _ => "--data (redacted)".into(),
            };
            timer.succeeded("", &summary, None);
        }
        Ok(Outcome::Closed) => {
            if let Request::Close { session } = request {
                timer.succeeded("", session, None);
            }
        }
        Ok(_) => {}
    }
}
