//! What makes a tunnel discoverable to the next process: its id, and the record
//! that names it.
//!
//! The record is a pointer, never the durable thing itself — the forward belongs
//! to its OpenSSH master (AGENTS.md §4). The record is committed before the
//! master starts, so a process exit during startup leaves a way to inspect a
//! possible forward.

use crate::config;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;

/// The id shape: `t_` plus 128 bits of hex. Interoperable with the reference,
/// and narrow enough that a hand-written path can never name another process'
/// socket by accident.
const ID_PREFIX: &str = "t_";
const ID_HEX: usize = 32;

/// The three forwards rhost builds, each one already an OpenSSH flag.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Local,
    Reverse,
    Socks,
}

impl Kind {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "local" => Some(Self::Local),
            "reverse" => Some(Self::Reverse),
            "socks" => Some(Self::Socks),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Reverse => "reverse",
            Self::Socks => "socks",
        }
    }

    /// The ssh flag this kind *is*: rhost has no separate vocabulary for it.
    pub fn flag(self) -> &'static str {
        match self {
            Self::Local => "-L",
            Self::Reverse => "-R",
            Self::Socks => "-D",
        }
    }
}

/// Whether the forward is still there. `alive` means the dedicated master
/// answered; `stale` means its socket is gone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Alive,
    Stale,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Alive => "alive",
            Self::Stale => "stale",
        }
    }
}

/// What a later process reads back from disk. v2 dropped the reference's
/// duplicate `id` field, so the record names the tunnel exactly once.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub tunnel_id: String,
    pub host: String,
    pub kind: Kind,
    pub listen: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination: Option<String>,
}

/// One tunnel with the status this invocation observed.
#[derive(Clone, Debug)]
pub struct Tunnel {
    pub id: String,
    pub host: String,
    pub kind: Kind,
    pub listen: String,
    pub destination: Option<String>,
    pub status: Status,
}

impl Tunnel {
    pub fn from_record(record: Record, status: Status) -> Self {
        Self {
            id: record.tunnel_id,
            host: record.host,
            kind: record.kind,
            listen: record.listen,
            destination: record.destination,
            status,
        }
    }

    /// The record without the status: what is durable is the forward's address,
    /// never a claim about whether it is up.
    pub fn record(&self) -> Record {
        Record {
            tunnel_id: self.id.clone(),
            host: self.host.clone(),
            kind: self.kind,
            listen: self.listen.clone(),
            destination: self.destination.clone(),
        }
    }
}

/// Why a tunnel operation had no answer. The three cases are the three things a
/// caller can do about it: fix the arguments, stop asking about an id this
/// machine does not have, or retry a transport-shaped problem.
#[derive(Debug)]
pub enum Fault {
    Invalid(String),
    NotFound(String),
    Transport(String),
    /// A side effect may still exist; repeating the operation could compound it.
    Uncertain(String),
}

/// The directory records live in, under this version's own state namespace.
pub fn root() -> PathBuf {
    config::v4_state_dir().join("tunnels")
}

/// One tunnel's own control socket. It lives in the control directory, whose
/// length is already budgeted for OpenSSH's `sun_path` limit, and is
/// deliberately not the shared `%C` socket: closing one tunnel must not drop the
/// connection `exec` and `session` traffic is using.
pub fn socket(id: &str) -> PathBuf {
    config::control_dir().join(id)
}

pub fn ensure_root() -> Result<(), Fault> {
    config::ensure_private_dir(&root())
        .map_err(|error| Fault::Transport(format!("tunnel state root: {error}")))
}

fn record_path(id: &str) -> PathBuf {
    root().join(format!("{id}.json"))
}

pub fn is_id(text: &str) -> bool {
    text.strip_prefix(ID_PREFIX).is_some_and(|hex| {
        hex.len() == ID_HEX
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}

/// A fresh id. The bytes come from the operating system's random source because
/// the id is what selects a socket to signal: guessing it must not be a way to
/// reach someone else's master.
pub fn new_id() -> Result<String, Fault> {
    let mut bytes = [0u8; ID_HEX / 2];
    fs::File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut bytes))
        .map_err(|error| Fault::Transport(format!("cannot read random bytes: {error}")))?;
    let mut hex = String::with_capacity(ID_HEX);
    for byte in bytes {
        hex.push_str(&format!("{byte:02x}"));
    }
    Ok(format!("{ID_PREFIX}{hex}"))
}

/// Every record on this machine, oldest name first. A missing directory is an
/// empty machine, not a failure: nothing has been opened yet.
pub fn ids() -> Result<Vec<String>, Fault> {
    let entries = match fs::read_dir(root()) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(Fault::Transport(format!(
                "read {}: {error}",
                root().display()
            )));
        }
    };
    let mut ids: Vec<String> = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|error| Fault::Transport(format!("read tunnel root: {error}")))?;
        if !entry.path().is_file() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // A file that is not named after a tunnel is not one: it belongs to
        // something else in this directory, or to nothing.
        if let Some(id) = name.strip_suffix(".json") {
            if is_id(id) {
                ids.push(id.to_string());
            }
        }
    }
    ids.sort();
    Ok(ids)
}

/// Reads one record and checks that the file is named after the tunnel it
/// describes, so a stray or hand-edited file cannot make rhost signal some other
/// process' socket.
pub fn read(id: &str) -> Result<Record, Fault> {
    if !is_id(id) {
        return Err(Fault::NotFound(format!("no such tunnel: {id}")));
    }
    let path = record_path(id);
    if is_symlink(&path) {
        return Err(Fault::Transport(format!("tunnel record {id} is a symlink")));
    }
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(Fault::NotFound(format!("no such tunnel: {id}")));
        }
        Err(error) => {
            return Err(Fault::Transport(format!(
                "read {}: {error}",
                path.display()
            )));
        }
    };
    let record: Record = serde_json::from_str(&text)
        .map_err(|error| Fault::Transport(format!("tunnel record {id} is unreadable: {error}")))?;
    if record.tunnel_id != id {
        return Err(Fault::Transport(format!(
            "tunnel record {id} describes {}",
            record.tunnel_id
        )));
    }
    Ok(record)
}

/// Publishes one complete user-only record before a master can start. A hard
/// link makes publication atomic and refuses to replace an existing id.
pub fn write(record: &Record) -> Result<(), Fault> {
    let path = record_path(&record.tunnel_id);
    if is_symlink(&path) {
        return Err(Fault::Transport(
            "tunnel record path is a symlink".to_string(),
        ));
    }
    let temporary = root().join(format!(".{}.tmp", record.tunnel_id));
    let mut body = serde_json::to_vec(record)
        .map_err(|error| Fault::Transport(format!("encode tunnel record: {error}")))?;
    body.push(b'\n');
    let result = (|| -> io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&body)?;
        file.flush()?;
        file.sync_all()?;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
        fs::hard_link(&temporary, &path)?;
        let _ = fs::remove_file(&temporary);
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(Fault::Transport(format!(
            "publish {}: {error}",
            path.display()
        )));
    }
    Ok(())
}

pub fn remove(id: &str) -> Result<(), Fault> {
    let path = record_path(id);
    fs::remove_file(&path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            Fault::NotFound(format!("no such tunnel: {id}"))
        } else {
            Fault::Transport(format!("remove {}: {error}", path.display()))
        }
    })
}

fn is_symlink(path: &std::path::Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|info| info.file_type().is_symlink())
}
