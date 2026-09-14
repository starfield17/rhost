//! The schema-v2 view of a tunnel.
//!
//! Tunnel DTOs live here rather than with the others because a tunnel is the one
//! result whose `status` is not rhost's claim but an observation: the DTO can
//! only be built from a value whose master has already been asked.

use super::Envelope;
use crate::tunnel;
use serde::Serialize;

/// One tunnel: the id a caller keeps, and what its own master just said.
#[derive(Debug, Serialize)]
pub struct TunnelDto<'a> {
    tunnel_id: &'a str,
    host: &'a str,
    kind: &'a str,
    listen: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    destination: Option<&'a str>,
    status: &'a str,
}

impl<'a> From<&'a tunnel::Tunnel> for TunnelDto<'a> {
    fn from(value: &'a tunnel::Tunnel) -> Self {
        Self {
            tunnel_id: &value.id,
            host: &value.host,
            kind: value.kind.as_str(),
            listen: &value.listen,
            // A socks forward has no destination, and an empty one would be a
            // second way to say the same thing.
            destination: value.destination.as_deref(),
            status: value.status.as_str(),
        }
    }
}

/// `tunnel.open` returns the record it just created, so a caller never has to
/// list to learn the id of the forward it asked for.
pub fn tunnel<'a>(
    operation: &'static str,
    host: &'a str,
    value: &'a tunnel::Tunnel,
) -> Envelope<TunnelDto<'a>> {
    Envelope {
        schema_version: 2,
        operation,
        ok: true,
        host: (!host.is_empty()).then(|| host.to_string()),
        data: TunnelDto::from(value),
        error: None,
    }
}

#[derive(Debug, Serialize)]
pub struct TunnelsDto<'a> {
    tunnels: Vec<TunnelDto<'a>>,
}

pub fn tunnels(rows: &[tunnel::Tunnel]) -> Envelope<TunnelsDto<'_>> {
    Envelope {
        schema_version: 2,
        operation: "tunnel.list",
        ok: true,
        host: None,
        data: TunnelsDto {
            tunnels: rows.iter().map(TunnelDto::from).collect(),
        },
        error: None,
    }
}

/// A closed tunnel is named by the id the caller passed, so the answer says
/// exactly which forward this call stopped.
#[derive(Debug, Serialize)]
pub struct TunnelClosedDto<'a> {
    tunnel_id: &'a str,
    closed: bool,
}

pub fn tunnel_closed(id: &str) -> Envelope<TunnelClosedDto<'_>> {
    Envelope {
        schema_version: 2,
        operation: "tunnel.close",
        ok: true,
        host: None,
        data: TunnelClosedDto {
            tunnel_id: id,
            closed: true,
        },
        error: None,
    }
}
