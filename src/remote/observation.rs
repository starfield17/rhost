//! A request-scoped observer; stopping it never signals a terminal pane.
use super::{Error, RemoteRun, exec};
use crate::transport::Client;
use std::time::Duration;

pub(crate) struct Observation<'a> {
    client: &'a Client,
    cancelled: &'a dyn Fn() -> bool,
}
impl<'a> Observation<'a> {
    pub fn new(client: &'a Client, cancelled: &'a dyn Fn() -> bool) -> Self {
        Self { client, cancelled }
    }
    pub fn check(&self) -> Result<(), Error> {
        if (self.cancelled)() {
            Err(Error::new(
                "REMOTE_COMMAND_CANCELLED",
                "observation cancelled; inspect remote state before retrying",
            )
            .with_recovery(crate::domain::Recovery::not_dispatched()))
        } else {
            Ok(())
        }
    }
    pub fn partial(
        &self,
        host: &str,
        command: &str,
        timeout: Duration,
        capture: usize,
    ) -> Result<RemoteRun, Error> {
        self.check()?;
        let request = exec::Request {
            host,
            command,
            cwd: None,
            env: &[],
            timeout: Some(timeout),
            capture: Some(capture),
            fresh: false,
        };
        let outcome = exec::observe_captured(self.client, command, &request, None, self.cancelled)
            .map_err(|e| Error::new(e.code(), e.to_string()))?;
        Ok(RemoteRun::from_outcome(&outcome))
    }
    pub fn run(
        &self,
        host: &str,
        command: &str,
        timeout: Duration,
        capture: usize,
    ) -> Result<(u8, String, String), Error> {
        let result = self.partial(host, command, timeout, capture)?;
        if let Some(error) = result.failure {
            return Err(error);
        }
        let code = result.status.ok_or_else(|| {
            Error::new("REMOTE_EXECUTION_UNKNOWN", "completion evidence is missing")
        })?;
        Ok((code, result.stdout, result.stderr))
    }
}
