//! The `hosts` envelope: what the local OpenSSH client config names.

use crate::host;
use crate::wire::Envelope;
use serde::Serialize;

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
