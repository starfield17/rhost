//! Ephemeral target discovery and deliberately distinct preparation profiles.
use super::{Helper, artifact, build};
use crate::remote::Error;
use std::time::Duration;
#[derive(Clone)]
pub(super) struct Target {
    pub os: String,
    pub arch: String,
}
pub(super) enum Preparation {
    Supplied(artifact::Binary),
    Local(build::Configuration),
    Automatic,
}
impl Helper<'_> {
    pub(super) fn cache_path(&self) -> String {
        self.selected_cache
            .borrow()
            .as_ref()
            .unwrap_or(&self.cache)
            .clone()
    }
    pub(super) fn select_profile(&self, profile: &str) -> Result<(), Error> {
        let t = self
            .target
            .get()
            .ok_or_else(|| Error::new("INTERNAL", "target was not observed"))?;
        let base = self.cache_base()?;
        *self.selected_cache.borrow_mut() = Some(format!("{}{}-{}/{profile}/", base, t.os, t.arch));
        Ok(())
    }
    fn cache_base(&self) -> Result<String, Error> {
        let Some(root) = std::env::var_os("RHOST_HELPER_CACHE_DIR") else {
            return Ok(self.cache.clone());
        };
        let root = root
            .into_string()
            .map_err(|_| Error::new("CONFIG_INVALID", "RHOST_HELPER_CACHE_DIR must be UTF-8"))?;
        crate::domain::RemotePath::new(root.clone())
            .map_err(|e| Error::new("CONFIG_INVALID", e.to_string()))?;
        if !(root.starts_with('/') || root == "~" || root.starts_with("~/"))
            || root.split('/').any(|p| p == "." || p == "..")
        {
            return Err(Error::new(
                "CONFIG_INVALID",
                "helper cache root must be absolute or home-relative without dot components",
            ));
        }
        Ok(format!(
            "{}/p1/{}/",
            root.trim_end_matches('/'),
            super::source_digest()
        ))
    }
    pub(super) fn initialize(&self) -> Result<(), Error> {
        if self.preparation.borrow().is_some() {
            return Ok(());
        }
        self.cache_base()?;
        let marker = format!(
            "RHOST_TARGET_{}\n",
            crate::random::hex(16).map_err(|e| Error::new("INTERNAL", e.to_string()))?
        );
        let script = format!(
            "printf '%s' {}; command uname -s; command uname -m",
            crate::shell::quote(&marker)
        );
        let (run, stdout, stderr) = self.transport(
            &format!("sh -c {}", crate::shell::quote(&script)),
            &[],
            Duration::from_secs(15),
            4096,
        )?;
        if run.cancelled {
            return Err(Error::new(
                "REMOTE_COMMAND_CANCELLED",
                "target observation cancelled",
            ));
        }
        if run.exit != Some(0) {
            let diagnostic = String::from_utf8_lossy(&stderr);
            if let Some(kind) = crate::remote::pre_execution(&diagnostic, run.exit) {
                use crate::domain::PreExecFailure as P;
                let code = match kind {
                    P::Usage => "USAGE_ERROR",
                    P::Config => "CONFIG_INVALID",
                    P::HostUnknown => "HOST_UNKNOWN",
                    P::Authentication => "SSH_AUTH_FAILED",
                    P::HostKey => "HOST_KEY_FAILED",
                    P::Connection => "SSH_UNREACHABLE",
                    P::DependencyMissing => "REMOTE_DEPENDENCY_MISSING",
                };
                let error = Error::new(code, format!("target observation failed: {diagnostic}"));
                return Err(if kind == P::Connection {
                    error.retryable()
                } else {
                    error
                });
            }
            return Err(Error::new(
                if run.exit == Some(127) {
                    "REMOTE_DEPENDENCY_MISSING"
                } else {
                    "REMOTE_EXECUTION_UNKNOWN"
                },
                format!("target observation failed without complete evidence: {diagnostic}"),
            ));
        }
        let start = stdout
            .windows(marker.len())
            .position(|w| w == marker.as_bytes())
            .ok_or_else(|| Error::new("PROTOCOL_ERROR", "target observation marker missing"))?
            + marker.len();
        let text = std::str::from_utf8(&stdout[start..])
            .map_err(|_| Error::new("PROTOCOL_ERROR", "target observation is not UTF-8"))?;
        let mut lines = text.lines();
        let os = match lines.next() {
            Some("Linux") => "linux",
            Some("Darwin") => "darwin",
            Some("FreeBSD") => "freebsd",
            Some("OpenBSD") => "openbsd",
            Some("NetBSD") => "netbsd",
            _ => {
                return Err(Error::new(
                    "UNSUPPORTED_REMOTE_OS",
                    "unsupported helper target",
                ));
            }
        };
        let arch = match lines.next() {
            Some("aarch64" | "arm64") => "aarch64",
            Some("x86_64" | "amd64") => "x86_64",
            _ => {
                return Err(Error::new(
                    "UNSUPPORTED_REMOTE_OS",
                    "unsupported helper architecture",
                ));
            }
        };
        if lines.next().is_some() {
            return Err(Error::new(
                "PROTOCOL_ERROR",
                "unexpected target observation fields",
            ));
        }
        let t = Target {
            os: os.into(),
            arch: arch.into(),
        };
        let _ = self.target.set(t.clone());
        let preparation = if let Some(path) = std::env::var_os("RHOST_HELPER_BIN") {
            let binary = artifact::Binary::read(&path)?;
            self.select_profile(&format!("supplied-{}", binary.sha256))?;
            Preparation::Supplied(binary)
        } else if std::env::var_os("RHOST_HELPER_CC").is_some() {
            let c = build::Configuration::load(&t, self.cancelled)?;
            self.select_profile(&format!("local-{}", c.identity))?;
            Preparation::Local(c)
        } else {
            self.select_profile("native-c99-o2-v1")?;
            Preparation::Automatic
        };
        *self.preparation.borrow_mut() = Some(preparation);
        Ok(())
    }
    pub(super) fn expected_image(&self) -> Result<Option<String>, Error> {
        if let Some(Preparation::Supplied(binary)) = self.preparation.borrow().as_ref() {
            return Ok(Some(binary.sha256.clone()));
        }
        if let Some(cache) = self.selected_cache.borrow().as_ref()
            && let Some(profile) = cache.trim_end_matches('/').rsplit('/').next()
            && let Some(digest) = profile.strip_prefix("release-")
        {
            crate::domain::Sha256::new(digest.into())
                .map_err(|_| Error::new("INTERNAL", "invalid release profile"))?;
            return Ok(Some(digest.into()));
        }
        Ok(None)
    }
    pub(super) fn automatic(&self) -> bool {
        matches!(
            self.preparation.borrow().as_ref(),
            Some(Preparation::Automatic)
        )
    }
}
