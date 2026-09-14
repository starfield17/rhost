//! The `hosts`, `connection` and `doctor` envelopes, including the view of the
//! local OpenSSH client that discovery could actually see.

use super::exec::exec;
use super::{Envelope, error};
use crate::domain;
use crate::{app, host, transport};
use serde::Serialize;

/// The `hosts` data: what the local OpenSSH client config names, and how much
/// of it this reader could actually see.
#[derive(Debug, Serialize)]
pub struct HostsDto {
    hosts: Vec<HostDto>,
    config_found: bool,
    complete: bool,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct HostDto {
    alias: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
}

pub fn hosts(discovery: &host::Discovery) -> Envelope<HostsDto> {
    Envelope {
        schema_version: 2,
        operation: "hosts",
        ok: true,
        host: None,
        error: None,
        data: HostsDto {
            hosts: discovery
                .hosts
                .iter()
                .map(|info| HostDto {
                    alias: info.alias.clone(),
                    source: info.source.clone(),
                })
                .collect(),
            config_found: discovery.config_found,
            complete: discovery.complete,
            warnings: discovery.warnings.clone(),
        },
    }
}

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

impl From<&app::doctor::Connection> for ConnectionDto {
    fn from(value: &app::doctor::Connection) -> Self {
        Self {
            master_status: value.master_status.as_str(),
            control_path: value.control_path.clone(),
            master_pid: value.master_pid,
            diagnostic: value.diagnostic.clone(),
            stopped: value.stopped,
        }
    }
}

impl From<&transport::openssh::ConnectionStatus> for ConnectionDto {
    fn from(value: &transport::openssh::ConnectionStatus) -> Self {
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

#[derive(Debug, Serialize)]
pub struct DoctorDto {
    host: String,
    os: String,
    kernel: String,
    arch: String,
    user: String,
    home: String,
    login_shell: String,
    state_dir: String,
    online: bool,
    state_dir_writable: bool,
    wsl: bool,
    capabilities: std::collections::BTreeMap<String, bool>,
    connection: ConnectionDto,
    connection_reused: Option<bool>,
}

pub fn doctor(report: &app::doctor::Report) -> Envelope<DoctorDto> {
    Envelope {
        schema_version: 2,
        operation: "doctor",
        ok: true,
        host: Some(report.host.clone()),
        error: None,
        data: DoctorDto {
            host: report.host.clone(),
            os: report.os.clone(),
            kernel: report.kernel.clone(),
            arch: report.arch.clone(),
            user: report.user.clone(),
            home: report.home.clone(),
            login_shell: report.login_shell.clone(),
            state_dir: report.state_dir.clone(),
            online: report.online,
            state_dir_writable: report.state_dir_writable,
            wsl: report.wsl,
            capabilities: report.capabilities.clone(),
            connection: ConnectionDto::from(&report.connection),
            connection_reused: report.connection_reused,
        },
    }
}

/// A doctor run whose probe failed still reports what it knew, so the data
/// object is delivered alongside the error.
pub fn doctor_failure(
    report: &app::doctor::Report,
    outcome: &domain::ExecOutcome,
) -> Envelope<DoctorDto> {
    let Envelope { error, .. } = exec(&report.host, outcome);
    Envelope {
        schema_version: 2,
        operation: "doctor",
        ok: false,
        host: Some(report.host.clone()),
        error,
        data: DoctorDto {
            host: report.host.clone(),
            os: report.os.clone(),
            kernel: report.kernel.clone(),
            arch: report.arch.clone(),
            user: report.user.clone(),
            home: report.home.clone(),
            login_shell: report.login_shell.clone(),
            state_dir: report.state_dir.clone(),
            online: report.online,
            state_dir_writable: report.state_dir_writable,
            wsl: report.wsl,
            capabilities: report.capabilities.clone(),
            connection: ConnectionDto::from(&report.connection),
            connection_reused: report.connection_reused,
        },
    }
}
