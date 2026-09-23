//! Tunnels: one port forward held by its own OpenSSH master.
//!
//! The CLI that opens a tunnel owns nothing durable. The forward belongs to a
//! dedicated `ControlMaster=yes ... -fNT` process, and the record on disk is only
//! how a later process finds that master again (AGENTS.md §4). Every tunnel
//! therefore survives the CLI exiting and does not survive a client reboot, and
//! `list` is how a caller discovers what is still up.
//!
//! Two decisions are made here rather than in the CLI, because they are
//! deliberate refusals rather than formatting: which binds are allowed to leave
//! loopback, and what the three answers about a master's state mean. The cut is
//! `record` for what is durable, `master` for the OpenSSH process, and this file
//! for the requests a caller can make.

pub mod command;
mod dto;
mod master;
mod record;
mod render;
mod run;

pub use command::{Command, command};
pub use dto::{tunnel, tunnel_closed, tunnels};
pub use record::{Fault, Kind, Status, Tunnel};
pub use run::run;

use crate::config;
use crate::transport::Client;
use master::{Control, Master};
use std::net::IpAddr;

/// One forward, as the caller asked for it and as rhost agreed to build it.
pub struct Request<'a> {
    pub host: &'a str,
    pub kind: Kind,
    pub listen: String,
    /// `None` for a socks forward, which has no destination to forward to.
    pub destination: Option<String>,
}

impl<'a> Request<'a> {
    /// Validates a forward without creating a directory, starting ssh or writing
    /// anything, so a request rhost would never build is refused before it can
    /// leave a trace.
    ///
    /// The two guards are the point of this function:
    ///
    ///  1. a listen address that is not loopback changes who else can reach the
    ///     remote network, so it takes an explicit `--allow-exposure` rather than
    ///     a default someone has to remember to change;
    ///  2. port 0 asks the operating system to choose, and rhost would have no
    ///     way to report the port it did not choose.
    pub fn new(
        host: &'a str,
        kind: &str,
        listen: &str,
        destination: &str,
        expose: bool,
    ) -> Result<Self, Fault> {
        let kind = Kind::parse(kind).ok_or_else(|| {
            Fault::Invalid(format!(
                "kind must be local, reverse or socks, not {kind:?}"
            ))
        })?;
        let (address, port) = split_host_port(listen).ok_or_else(|| {
            Fault::Invalid(format!("--listen must be address:port, not {listen:?}"))
        })?;
        if port == 0 {
            return Err(Fault::Invalid(
                "--listen port must be explicit; rhost cannot report a port it did not choose"
                    .to_string(),
            ));
        }
        let destination = match kind {
            Kind::Socks => {
                if !destination.is_empty() {
                    return Err(Fault::Invalid(
                        "--destination is not used for a socks forward".to_string(),
                    ));
                }
                None
            }
            Kind::Local | Kind::Reverse => {
                split_host_port(destination).ok_or_else(|| {
                    Fault::Invalid(format!(
                        "--destination must be host:port for a {kind} forward",
                        kind = kind.as_str()
                    ))
                })?;
                Some(destination.to_string())
            }
        };
        if !expose && !is_loopback(address) {
            return Err(Fault::Invalid(format!(
                "binding {listen} is reachable from elsewhere; pass --allow-exposure to mean it"
            )));
        }
        Ok(Self {
            host,
            kind,
            listen: listen.to_string(),
            destination,
        })
    }

    /// The argument ssh's own forward flag takes: `listen:destination` for a
    /// directed forward, and the listen address alone for socks.
    fn forward(&self) -> String {
        match &self.destination {
            Some(destination) => format!("{}:{destination}", self.listen),
            None => self.listen.clone(),
        }
    }
}

/// Opens one forward on its own master and returns the record it created.
pub fn open(client: &Client, request: &Request<'_>) -> Result<Tunnel, Fault> {
    let id = record::new_id()?;
    // The socket directory has to exist before ssh can create its socket in it.
    config::ensure_control_dir().map_err(|error| Fault::Transport(error.to_string()))?;
    record::ensure_root()?;
    let socket = record::socket(&id);
    let options = client.options(false);
    let master = Master {
        ssh_bin: client.ssh_bin(),
        options: &options,
    };
    let tunnel = Tunnel {
        id,
        host: request.host.to_string(),
        kind: request.kind,
        listen: request.listen.clone(),
        destination: request.destination.clone(),
        status: Status::Alive,
    };
    record::write(&tunnel.record())?;
    match master.start(
        &socket,
        request.kind.flag(),
        &request.forward(),
        request.host,
    ) {
        Ok(()) => Ok(tunnel),
        Err(start_error) => match master.stop(&socket, request.host) {
            Ok(()) if !matches!(&start_error, Fault::Uncertain(_)) => {
                record::remove(&tunnel.id).map_err(|error| {
                    Fault::Uncertain(format!(
                        "tunnel {} did not start, but its record could not be removed: {error:?}",
                        tunnel.id
                    ))
                })?;
                Err(start_error)
            }
            Ok(()) => Err(Fault::Uncertain(format!(
                "tunnel {} startup was uncertain; its record remains for inspection",
                tunnel.id
            ))),
            Err(reason) => Err(Fault::Uncertain(format!(
                "tunnel {} may still be running after startup failed: {reason}",
                tunnel.id
            ))),
        },
    }
}

/// Every tunnel this machine has a record for, each one with the answer its own
/// master just gave.
///
/// A record whose socket is gone is reported as `stale` and left in place: it is
/// evidence, and something else may still be using the forward's slot.
pub fn list(client: &Client) -> Result<Vec<Tunnel>, Fault> {
    let options = client.options(false);
    let master = Master {
        ssh_bin: client.ssh_bin(),
        options: &options,
    };
    let mut tunnels: Vec<Tunnel> = Vec::new();
    for id in record::ids()? {
        let stored = record::read(&id)?;
        let status = match master.check(&record::socket(&stored.tunnel_id), &stored.host) {
            Control::Answered => Status::Alive,
            Control::SocketAbsent => Status::Stale,
            // An unresponsive socket leaves the master's state unknown, and
            // "unknown" is not a status this list is allowed to invent.
            Control::Uncertain(reason) => return Err(Fault::Transport(reason)),
        };
        tunnels.push(Tunnel::from_record(stored, status));
    }
    Ok(tunnels)
}

/// Stops exactly one tunnel — its master and its record — and returns the id it
/// closed. A record whose master is already gone is still closed: the point is
/// to leave nothing half-remembered behind.
pub fn close(client: &Client, id: &str) -> Result<String, Fault> {
    let stored = record::read(id)?;
    let options = client.options(false);
    let master = Master {
        ssh_bin: client.ssh_bin(),
        options: &options,
    };
    let socket = record::socket(&stored.tunnel_id);
    match master.check(&socket, &stored.host) {
        Control::Answered => {
            master
                .stop(&socket, &stored.host)
                .map_err(Fault::Uncertain)?;
        }
        Control::SocketAbsent => {}
        Control::Uncertain(reason) => return Err(Fault::Transport(reason)),
    }
    record::remove(&stored.tunnel_id)?;
    Ok(stored.tunnel_id)
}

/// `address:port`, with an IPv6 address still bracketed. A missing port, an
/// empty one and a port outside the range are all the same answer: not an
/// address ssh could bind.
fn split_host_port(text: &str) -> Option<(&str, u16)> {
    let (address, port) = text.rsplit_once(':')?;
    Some((address, port.parse().ok()?))
}

/// The exposure guard. `localhost` is accepted by name because that is what a
/// person types; an explicit loopback address is the same promise, since the
/// kernel will not accept the connection from another machine.
fn is_loopback(address: &str) -> bool {
    if address == "localhost" {
        return true;
    }
    let bare = address
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(address);
    bare.parse::<IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_the_reference_shape_and_fresh_each_time() {
        let first = record::new_id().unwrap_or_else(|_| panic!("random source"));
        let second = record::new_id().unwrap_or_else(|_| panic!("random source"));
        assert!(record::is_id(&first) && record::is_id(&second), "{first}");
        assert_ne!(first, second);
        assert!(!record::is_id("t_AB") && !record::is_id(&first[..33]));
    }

    #[test]
    fn a_socks_forward_carries_no_destination_and_the_others_must_have_one() {
        let socks = Request::new("gpu", "socks", "localhost:1080", "", false)
            .unwrap_or_else(|_| panic!("socks binds loopback"));
        assert_eq!(socks.forward(), "localhost:1080");
        assert!(Request::new("gpu", "socks", "localhost:1080", "localhost:80", false).is_err());
        assert!(Request::new("gpu", "local", "localhost:8080", "", false).is_err());
        let local = Request::new("gpu", "local", "localhost:8080", "localhost:80", false)
            .unwrap_or_else(|_| panic!("local binds loopback"));
        assert_eq!(local.forward(), "localhost:8080:localhost:80");
    }

    #[test]
    fn only_loopback_binds_are_free_and_the_rest_have_to_be_asked_for() {
        for allowed in ["localhost:80", "127.0.0.1:80", "[::1]:80"] {
            assert!(
                Request::new("gpu", "local", allowed, "localhost:80", false).is_ok(),
                "{allowed}"
            );
        }
        for refused in ["0.0.0.0:80", "[::]:80", "192.0.2.9:80", ":80"] {
            assert!(
                Request::new("gpu", "local", refused, "localhost:80", false).is_err(),
                "{refused} must need --allow-exposure"
            );
            assert!(
                Request::new("gpu", "local", refused, "localhost:80", true).is_ok(),
                "{refused} with --allow-exposure"
            );
        }
    }

    #[test]
    fn portless_and_zero_ports_are_refused_before_ssh_sees_them() {
        for bad in ["localhost", "localhost:", "localhost:0", "localhost:no"] {
            assert!(
                Request::new("gpu", "local", bad, "localhost:80", false).is_err(),
                "{bad}"
            );
        }
        assert!(Request::new("gpu", "unknown", "localhost:80", "localhost:80", false).is_err());
    }
}
