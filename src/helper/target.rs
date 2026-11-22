//! Ephemeral target discovery and deliberately distinct preparation profiles.
use super::{Helper, artifact, build};
use crate::{domain::Sha256, remote::Error};
use std::{path::Path, time::Duration};

pub(super) struct Target {
    pub os: String,
    pub arch: String,
}
impl Target {
    pub fn abi(&self) -> String {
        format!("{}-{}", self.os, self.arch)
    }
    pub fn explicit(value: &str) -> Result<Self, Error> {
        match value.split_once('-') {
            Some((
                os @ ("linux" | "darwin" | "freebsd" | "openbsd" | "netbsd"),
                arch @ ("aarch64" | "x86_64"),
            ))
            | Some((os @ "linux", arch @ "armv7-eabihf")) => Ok(Self {
                os: os.into(),
                arch: arch.into(),
            }),
            _ => Err(Error::new("CONFIG_INVALID", "unsupported RHOST_TARGET_ABI")),
        }
    }
    pub fn observed(text: &str, explicit: Option<Self>) -> Result<Self, Error> {
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
        let target = if let Some(target) = explicit {
            if target.os != os {
                return Err(Error::new(
                    "HELPER_INCOMPATIBLE",
                    "RHOST_TARGET_ABI does not match the remote OS",
                ));
            }
            target
        } else {
            let arch = match lines.next() {
                Some("aarch64" | "arm64") => "aarch64",
                Some("x86_64" | "amd64") => "x86_64",
                _ => {
                    return Err(Error::new(
                        "UNSUPPORTED_REMOTE_OS",
                        "unsupported helper architecture; set RHOST_TARGET_ABI for an explicit ABI",
                    ));
                }
            };
            Self {
                os: os.into(),
                arch: arch.into(),
            }
        };
        if lines.next().is_some() {
            return Err(Error::new(
                "PROTOCOL_ERROR",
                "unexpected target observation fields",
            ));
        }
        Ok(target)
    }
}

pub(super) struct Fixed {
    pub path: String,
    pub pin: Option<Sha256>,
}
impl Fixed {
    pub fn new(path: String, pin: Option<String>) -> Result<Self, Error> {
        crate::domain::RemotePath::new(path.clone())
            .map_err(|e| Error::new("CONFIG_INVALID", e.to_string()))?;
        if !(path.starts_with('/') || path.starts_with("~/"))
            || path.ends_with('/')
            || path.contains("//")
            || path.split('/').any(|p| p == "." || p == "..")
        {
            return Err(Error::new(
                "CONFIG_INVALID",
                "RHOST_REMOTE_HELPER_PATH must be absolute or home-relative without dot components",
            ));
        }
        let pin = pin
            .map(Sha256::new)
            .transpose()
            .map_err(|e| Error::new("CONFIG_INVALID", format!("RHOST_HELPER_SHA256: {e}")))?;
        Ok(Self { path, pin })
    }
}

pub(super) enum Preparation {
    Fixed(Fixed),
    Supplied(artifact::Binary),
    Local(build::Configuration),
    Automatic,
}

fn utf8_env(name: &str) -> Result<Option<String>, Error> {
    std::env::var_os(name)
        .map(|value| {
            value
                .into_string()
                .map_err(|_| Error::new("CONFIG_INVALID", format!("{name} must be UTF-8")))
        })
        .transpose()
}

impl Helper<'_> {
    pub(super) fn explicit_selection() -> bool {
        [
            "RHOST_TARGET_ABI",
            "RHOST_HELPER_PACK_DIR",
            "RHOST_REMOTE_HELPER_PATH",
            "RHOST_HELPER_SHA256",
        ]
        .iter()
        .any(|name| std::env::var_os(name).is_some())
    }
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
        *self.selected_cache.borrow_mut() = Some(format!("{}{}/{profile}/", base, t.abi()));
        Ok(())
    }
    pub(super) fn cache_base(&self) -> Result<String, Error> {
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
        let explicit = utf8_env("RHOST_TARGET_ABI")?
            .as_deref()
            .map(Target::explicit)
            .transpose()?;
        let path = utf8_env("RHOST_REMOTE_HELPER_PATH")?;
        let pin = utf8_env("RHOST_HELPER_SHA256")?;
        if pin.is_some() && path.is_none() {
            return Err(Error::new(
                "CONFIG_INVALID",
                "RHOST_HELPER_SHA256 requires RHOST_REMOTE_HELPER_PATH",
            ));
        }
        let fixed = path.map(|path| Fixed::new(path, pin)).transpose()?;
        if fixed.is_none() {
            self.cache_base()?;
        }
        let t = self.observe_target(explicit)?;
        let pack = std::env::var_os("RHOST_HELPER_PACK_DIR")
            .map(|root| artifact::from_pack(Path::new(&root), &t))
            .transpose()?;
        let _ = self.target.set(t);
        let preparation = if let Some(mut fixed) = fixed {
            if let Some(binary) = pack {
                if fixed
                    .pin
                    .as_ref()
                    .is_some_and(|pin| pin.as_str() != binary.sha256)
                {
                    return Err(Error::new(
                        "HELPER_INCOMPATIBLE",
                        "helper pin and pack SHA-256 disagree",
                    ));
                }
                fixed.pin = Some(
                    Sha256::new(binary.sha256)
                        .map_err(|e| Error::new("INTERNAL", e.to_string()))?,
                );
            }
            Preparation::Fixed(fixed)
        } else if let Some(binary) = pack {
            self.select_profile(&format!("supplied-{}", binary.sha256))?;
            Preparation::Supplied(binary)
        } else if let Some(path) = std::env::var_os("RHOST_HELPER_BIN") {
            let binary = artifact::Binary::read(&path)?;
            self.select_profile(&format!("supplied-{}", binary.sha256))?;
            Preparation::Supplied(binary)
        } else if std::env::var_os("RHOST_HELPER_CC").is_some() {
            let c = build::Configuration::load(
                self.target
                    .get()
                    .ok_or_else(|| Error::new("INTERNAL", "target missing"))?,
                self.cancelled,
            )?;
            self.select_profile(&format!("local-{}", c.identity))?;
            Preparation::Local(c)
        } else {
            self.select_profile("native-c99-o2-v1")?;
            Preparation::Automatic
        };
        *self.preparation.borrow_mut() = Some(preparation);
        Ok(())
    }
    fn observe_target(&self, explicit: Option<Target>) -> Result<Target, Error> {
        let marker = format!(
            "RHOST_TARGET_{}\n",
            crate::random::hex(16).map_err(|e| Error::new("INTERNAL", e.to_string()))?
        );
        let script = format!(
            "printf '%s' {}; command uname -s{}",
            crate::shell::quote(&marker),
            if explicit.is_some() {
                ""
            } else {
                " && command uname -m"
            }
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
        Target::observed(text, explicit)
    }
    pub(super) fn expected_image(&self) -> Result<Option<String>, Error> {
        match self.preparation.borrow().as_ref() {
            Some(Preparation::Supplied(binary)) => return Ok(Some(binary.sha256.clone())),
            Some(Preparation::Fixed(fixed)) => {
                return Ok(fixed.pin.as_ref().map(|pin| pin.as_str().to_owned()));
            }
            _ => (),
        }
        if let Some(cache) = self.selected_cache.borrow().as_ref()
            && let Some(profile) = cache.trim_end_matches('/').rsplit('/').next()
            && let Some(digest) = profile.strip_prefix(artifact::PROFILE_PREFIX)
        {
            crate::domain::Sha256::new(digest.into())
                .map_err(|_| Error::new("INTERNAL", "invalid release profile"))?;
            return Ok(Some(digest.into()));
        }
        Ok(None)
    }
    pub(super) fn fixed(&self) -> bool {
        matches!(
            self.preparation.borrow().as_ref(),
            Some(Preparation::Fixed(_))
        )
    }
    pub(super) fn automatic(&self) -> bool {
        matches!(
            self.preparation.borrow().as_ref(),
            Some(Preparation::Automatic)
        )
    }
}
