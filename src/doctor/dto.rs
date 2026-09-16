//! The `doctor` envelope: what a probe reported about a host.

use crate::connection::ConnectionDto;
use crate::domain;
use crate::wire::Envelope;
use crate::wire::execution::exec;
use serde::Serialize;

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
    capability_paths: std::collections::BTreeMap<String, Option<String>>,
    connection: ConnectionDto,
    connection_reused: Option<bool>,
}

pub fn doctor(report: &super::app::Report) -> Envelope<DoctorDto> {
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
            capability_paths: report.capability_paths.clone(),
            connection: ConnectionDto::from_parts(
                report.connection.master_status.as_str(),
                report.connection.control_path.clone(),
                report.connection.master_pid,
                report.connection.diagnostic.clone(),
                report.connection.stopped,
            ),
            connection_reused: report.connection_reused,
        },
    }
}

/// A doctor run whose probe failed still reports what it knew, so the data
/// object is delivered alongside the error.
pub fn doctor_failure(
    report: &super::app::Report,
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
            capability_paths: report.capability_paths.clone(),
            connection: ConnectionDto::from_parts(
                report.connection.master_status.as_str(),
                report.connection.control_path.clone(),
                report.connection.master_pid,
                report.connection.diagnostic.clone(),
                report.connection.stopped,
            ),
            connection_reused: report.connection_reused,
        },
    }
}
