//! The schema-v2 view of a control master's state, shared by `connection` and
//! `doctor` (which reports the same shape as part of a probe).

use crate::transport;
use crate::wire::{Envelope, error};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ConnectionDto {
    master_status: &'static str,
    control_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    master_pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    diagnostic: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stopped: bool,
}

impl ConnectionDto {
    pub fn from_parts(
        master_status: &'static str,
        control_path: String,
        master_pid: Option<u32>,
        diagnostic: Option<String>,
        stopped: bool,
    ) -> Self {
        Self {
            master_status,
            control_path,
            master_pid,
            diagnostic,
            stopped,
        }
    }
}

impl From<&transport::ConnectionStatus> for ConnectionDto {
    fn from(value: &transport::ConnectionStatus) -> Self {
        Self {
            master_status: value.master_status.as_str(),
            control_path: value.control_path.clone(),
            master_pid: value.master_pid,
            diagnostic: (!value.diagnostic.is_empty()).then(|| value.diagnostic.clone()),
            stopped: value.stopped,
        }
    }
}

/// `connection.status` and `connection.reset` share one data shape.
pub fn connection(
    operation: &'static str,
    host: &str,
    status: &ConnectionDto,
) -> Envelope<ConnectionDto> {
    Envelope {
        schema_version: 2,
        operation,
        ok: true,
        host: Some(host.to_string()),
        error: None,
        data: status.clone(),
    }
}

pub fn connection_failure(
    host: &str,
    status: &ConnectionDto,
    reason: &str,
) -> Envelope<ConnectionDto> {
    Envelope {
        schema_version: 2,
        operation: "connection.reset",
        ok: false,
        host: Some(host.to_string()),
        error: Some(error("SSH_CONTROL_FAILED", reason, false)),
        data: status.clone(),
    }
}
