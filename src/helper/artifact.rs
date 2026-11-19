//! Released helper bytes are accepted only after HTTPS and SHA-256 verification.
use super::{codec, target::Target};
use crate::{
    remote::Error,
    transport::{self, Recorder, Spec, StdinSource, Stream},
};
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
const VERSION: &str = env!("CARGO_PKG_VERSION");
pub(super) const PROFILE_PREFIX: &str = concat!("release-v", env!("CARGO_PKG_VERSION"), "-");

pub(super) fn supported(target: &Target) -> bool {
    target.os == "linux" && matches!(target.arch.as_str(), "x86_64" | "aarch64")
}

fn checksum(bytes: &[u8], asset: &str) -> Result<String, Error> {
    let digest = std::str::from_utf8(bytes).ok().and_then(|text| {
        let text = text.strip_suffix('\n').unwrap_or(text);
        let (hash, name) = text.split_once("  ")?;
        if name != asset {
            return None;
        }
        let hash = hash.to_ascii_lowercase();
        crate::domain::Sha256::new(hash.clone()).ok()?;
        Some(hash)
    });
    digest.ok_or_else(|| Error::new("HELPER_INCOMPATIBLE", "invalid helper SHA-256 sidecar"))
}

struct Download {
    bytes: Vec<u8>,
    limit: usize,
}
impl Stream for Download {
    fn feed(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("helper download exceeds size limit"));
        }
        self.bytes.extend(bytes);
        Ok(())
    }
}
fn download(url: String, limit: usize, cancelled: &dyn Fn() -> bool) -> Result<Vec<u8>, Error> {
    let args = [
        "--disable".into(),
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
        limit,
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
            "helper release download failed",
        ));
    }
    Ok(output.bytes)
}

pub(super) fn fetch(target: &Target, cancelled: &dyn Fn() -> bool) -> Result<Binary, Error> {
    if std::env::var_os("RHOST_NO_DOWNLOAD").is_some() {
        return Err(Error::new(
            "HELPER_PREPARATION_FAILED",
            "helper artifact downloads are disabled",
        ));
    }
    let asset = format!("rhost-helper_{VERSION}_{}-{}", target.os, target.arch);
    let url = format!("https://github.com/starfield17/rhost/releases/download/v{VERSION}/{asset}");
    let expected = checksum(&download(format!("{url}.sha256"), 4096, cancelled)?, &asset)?;
    let binary = Binary::bytes(download(url, codec::LIMIT, cancelled)?)
        .map_err(|error| Error::new("HELPER_INCOMPATIBLE", error.message))?;
    if binary.sha256 != expected {
        return Err(Error::new(
            "HELPER_INCOMPATIBLE",
            "helper download does not match its SHA-256 sidecar",
        ));
    }
    Ok(binary)
}
