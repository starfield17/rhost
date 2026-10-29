//! Released helper bytes are accepted only against the compiled distribution lock.
use super::{codec, target::Target};
use crate::{
    remote::Error,
    transport::{self, Recorder, Spec, StdinSource, Stream},
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    io::{self, Read},
    time::{Duration, Instant},
};
pub(super) struct Binary {
    pub bytes: Vec<u8>,
    pub sha256: String,
}
impl Binary {
    pub fn bytes(bytes: Vec<u8>) -> Result<Self, Error> {
        if bytes.is_empty() || bytes.len() > codec::LIMIT {
            return Err(Error::new(
                "CONFIG_INVALID",
                "helper binary must contain 1..8 MiB",
            ));
        }
        let sha256 = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        Ok(Self { bytes, sha256 })
    }
    pub fn read(path: &std::ffi::OsStr) -> Result<Self, Error> {
        let info = std::fs::symlink_metadata(path)
            .map_err(|e| Error::new("CONFIG_INVALID", e.to_string()))?;
        if !info.is_file() || info.file_type().is_symlink() {
            return Err(Error::new(
                "CONFIG_INVALID",
                "helper must be a regular local file",
            ));
        }
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .and_then(|f| f.take((codec::LIMIT + 1) as u64).read_to_end(&mut bytes))
            .map_err(|e| Error::new("CONFIG_INVALID", e.to_string()))?;
        Self::bytes(bytes)
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: u8,
    source_digest: String,
    release: String,
    toolchain: serde_json::Value,
    artifacts: Vec<Pinned>,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Pinned {
    pub target: String,
    pub asset: String,
    pub sha256: String,
    pub size: usize,
    pub role: String,
    pub build_config: String,
}
pub(super) fn pinned(target: &Target) -> Result<Option<(String, Pinned)>, Error> {
    let m: Manifest = serde_json::from_str(include_str!("../../helper/artifacts.json"))
        .map_err(|_| Error::new("INTERNAL", "invalid compiled helper lock"))?;
    if m.format != 1 {
        return Err(Error::new("INTERNAL", "unsupported helper lock"));
    }
    let _ = &m.toolchain;
    if m.source_digest != super::source_digest() {
        return Ok(None);
    }
    if m.release != format!("v{}", env!("CARGO_PKG_VERSION")) {
        return Err(Error::new(
            "INTERNAL",
            "helper release does not match client",
        ));
    }
    let name = format!("{}-{}", target.os, target.arch);
    let choices = m
        .artifacts
        .into_iter()
        .filter(|a| a.target == name && a.role == "runtime")
        .collect::<Vec<_>>();
    if choices.len() > 1 {
        return Err(Error::new("INTERNAL", "ambiguous helper lock"));
    }
    let Some(a) = choices.into_iter().next() else {
        return Ok(None);
    };
    if !a
        .asset
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
        || a.asset.is_empty()
        || !(1..=codec::LIMIT).contains(&a.size)
        || crate::domain::Sha256::new(a.sha256.clone()).is_err()
        || crate::domain::Sha256::new(a.build_config.clone()).is_err()
    {
        return Err(Error::new("INTERNAL", "invalid pinned helper artifact"));
    }
    Ok(Some((m.release, a)))
}
struct Download {
    bytes: Vec<u8>,
    limit: usize,
}
impl Stream for Download {
    fn feed(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("helper download exceeds pinned size"));
        }
        self.bytes.extend(bytes);
        Ok(())
    }
}
pub(super) fn fetch(
    release: &str,
    pinned: &Pinned,
    cancelled: &dyn Fn() -> bool,
) -> Result<Binary, Error> {
    if std::env::var_os("RHOST_NO_DOWNLOAD").is_some() {
        return Err(Error::new(
            "HELPER_PREPARATION_FAILED",
            "helper artifact downloads are disabled",
        ));
    }
    let url = format!(
        "https://github.com/starfield17/rhost/releases/download/{release}/{}",
        pinned.asset
    );
    let args = [
        "--proto".into(),
        "=https".into(),
        "--proto-redir".into(),
        "=https".into(),
        "--location".into(),
        "--fail".into(),
        "--silent".into(),
        "--show-error".into(),
        "--max-time".into(),
        "60".into(),
        url,
    ];
    let mut output = Download {
        bytes: Vec::new(),
        limit: pinned.size,
    };
    let mut error = Recorder::new(4096);
    let run = transport::run(
        &Spec {
            program: "curl",
            args: &args,
            stdin: StdinSource::Closed,
            deadline: Some(Instant::now() + Duration::from_secs(60)),
            cancelled,
            group: true,
        },
        &mut output,
        &mut error,
    );
    if run.cancelled {
        return Err(Error::new(
            "REMOTE_COMMAND_CANCELLED",
            "helper download cancelled",
        ));
    }
    if run.exit != Some(0) || run.failure.is_some() {
        return Err(Error::new(
            "HELPER_PREPARATION_FAILED",
            "pinned helper download failed",
        ));
    }
    let binary = Binary::bytes(output.bytes)?;
    if binary.bytes.len() != pinned.size || binary.sha256 != pinned.sha256 {
        return Err(Error::new(
            "HELPER_INCOMPATIBLE",
            "download does not match pinned helper bytes",
        ));
    }
    Ok(binary)
}
