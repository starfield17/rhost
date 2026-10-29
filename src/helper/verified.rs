//! A single SSH channel verifies the loaded image before capability dispatch.
use super::{Helper, Writer, artifact, build, codec, target::Target};
use crate::{domain::Recovery, remote::Error};
use serde_json::Value;
use std::time::Duration;

enum Attempt {
    Missing,
    Answer(Value, Vec<u8>),
}
impl Helper<'_> {
    fn profiles(&self) -> Result<Vec<(String, String, String)>, Error> {
        let supplied = std::env::var_os("RHOST_HELPER_BIN")
            .map(|path| artifact::Binary::read(&path))
            .transpose()?;
        let local = if supplied.is_none() && std::env::var_os("RHOST_HELPER_CC").is_some() {
            Some(build::Configuration::load(
                &Target {
                    os: "linux".into(),
                    arch: "aarch64".into(),
                },
                self.cancelled,
            )?)
        } else {
            None
        };
        let mut profiles = Vec::new();
        for os in ["linux", "darwin", "freebsd", "openbsd", "netbsd"] {
            for arch in ["aarch64", "x86_64"] {
                let target = Target {
                    os: os.into(),
                    arch: arch.into(),
                };
                let name = format!("{os}-{arch}");
                if let Some(binary) = &supplied {
                    profiles.push((
                        name,
                        format!("supplied-{}", binary.sha256),
                        binary.sha256.clone(),
                    ));
                } else if let Some(configuration) = &local {
                    if os == "linux" {
                        profiles.push((
                            name,
                            format!("local-{}", configuration.identity_for(arch)),
                            String::new(),
                        ));
                    }
                } else {
                    profiles.push((name.clone(), "native-c99-o2-v1".into(), String::new()));
                    if let Some((_, pinned)) = artifact::pinned(&target)? {
                        profiles.push((name, format!("release-{}", pinned.sha256), pinned.sha256));
                    }
                }
            }
        }
        Ok(profiles)
    }
    fn verified_command(&self, marker: &str, files: bool) -> Result<String, Error> {
        let profiles = self.profiles()?;
        let base = crate::shell::path_quote(&self.cache_base()?);
        let state = crate::shell::path_quote(
            &std::env::var("RHOST_REMOTE_STATE")
                .unwrap_or_else(|_| "~/.local/state/rhost/v5".into()),
        );
        let mut script = format!(
            "printf '%s' {}; rh_os=$(command uname -s) && rh_arch=$(command uname -m) || {{ printf 'RHOST_FAST_DEPENDENCY\\n'; exit 127; }};\ncase \"$rh_os\" in Linux) rh_os=linux;; Darwin) rh_os=darwin;; FreeBSD) rh_os=freebsd;; OpenBSD) rh_os=openbsd;; NetBSD) rh_os=netbsd;; *) printf 'RHOST_FAST_UNSUPPORTED\\n'; exit 126;; esac;\ncase \"$rh_arch\" in aarch64|arm64) rh_arch=aarch64;; x86_64|amd64) rh_arch=x86_64;; *) printf 'RHOST_FAST_UNSUPPORTED\\n'; exit 126;; esac;\nrh_target=\"$rh_os-$rh_arch\"; rh_base={base}; rh_cache=; rh_image=;\n",
            crate::shell::quote(marker)
        );
        for (target, profile, image) in profiles {
            script.push_str(&format!(
                "if [ -z \"$rh_cache\" ] && [ \"$rh_target\" = {} ] && {{ [ -e \"${{rh_base}}{target}/{profile}/rhost-helper\" ] || [ -L \"${{rh_base}}{target}/{profile}/rhost-helper\" ]; }}; then rh_cache=\"${{rh_base}}{target}/{profile}\"; rh_image={}; fi;\n",
                crate::shell::quote(&target), crate::shell::quote(&image)
            ));
        }
        script.push_str(
            "if [ -z \"$rh_cache\" ]; then printf 'RHOST_FAST_MISSING\\n'; exit 127; fi;\n",
        );
        script.push_str(&Self::guard("\"$rh_cache\""));
        script.push_str(&format!(
            "exec env RHOST_REMOTE_STATE={state} RHOST_EXPECT_BOOT_ID={} \"$rh_cache/rhost-helper\" {} {} \"$rh_image\" \"$rh_target\"",
            crate::shell::quote(self.boot_guard.map(|b| b.as_str()).unwrap_or("")),
            if files { "--verified-files" } else { "--verified-run" },
            crate::shell::quote(&super::source_digest())
        ));
        Ok(format!("sh -c {}", crate::shell::quote(&script)))
    }
    fn attempt(
        &self,
        kind: u32,
        input: &[u8],
        budget: Duration,
        cap: usize,
        files: bool,
    ) -> Result<Attempt, Error> {
        let nonce = crate::random::hex(16).map_err(|e| Error::new("INTERNAL", e.to_string()))?;
        let marker = if files {
            format!("RHOST_FILE_{nonce}\n")
        } else {
            format!("RHOST_FRAME_{kind}_{nonce}\n")
        };
        let command = self
            .verified_command(&marker, files)
            .map_err(|e| e.with_recovery(Recovery::preparation()))?;
        let (run, stdout, stderr) =
            self.transport(&command, input, budget, cap.saturating_add(65536))?;
        let at = stdout
            .windows(marker.len())
            .position(|s| s == marker.as_bytes())
            .map(|p| p + marker.len());
        if let Some(at) = at {
            let bytes = &stdout[at..];
            if bytes == b"RHOST_FAST_MISSING\n" {
                return Ok(Attempt::Missing);
            }
            for (receipt, code) in [
                (
                    b"RHOST_FAST_DEPENDENCY\n".as_slice(),
                    "REMOTE_DEPENDENCY_MISSING",
                ),
                (
                    b"RHOST_FAST_UNSUPPORTED\n".as_slice(),
                    "UNSUPPORTED_REMOTE_OS",
                ),
            ] {
                if bytes == receipt {
                    return Err(Error::new(code, "native bootstrap refused before dispatch")
                        .with_recovery(Recovery::preparation()));
                }
            }
            if bytes.len() >= 16 && &bytes[..4] == b"RH01" {
                let size = u64::from_be_bytes(
                    bytes[8..16]
                        .try_into()
                        .map_err(|_| codec::protocol("handshake length"))?,
                );
                if size > codec::LIMIT as u64 || size > bytes.len().saturating_sub(16) as u64 {
                    return Err(if run.cancelled {
                        cancelled()
                    } else {
                        codec::protocol("incomplete verified handshake")
                    });
                }
                let split = 16 + size as usize;
                let handshake = codec::answer(&bytes[..split], 1).map_err(|error| {
                    if matches!(error.code, "HELPER_INCOMPATIBLE" | "STATE_UNAVAILABLE")
                        && error.recovery.submission() == "confirmed"
                    {
                        error.with_recovery(Recovery::not_dispatched())
                    } else {
                        error
                    }
                })?;
                let identity = self.decode_probe(&handshake)?;
                let reply = bytes[split..].to_vec();
                if files {
                    if serde_json::from_slice::<Value>(&reply).is_err() {
                        return Err(if run.cancelled {
                            cancelled()
                        } else {
                            codec::protocol("incomplete verified file answer")
                        });
                    }
                } else {
                    return codec::answer(&reply, kind)
                        .map_err(|e| {
                            if run.cancelled && e.recovery.stage() == "unknown" {
                                cancelled()
                            } else {
                                e
                            }
                        })
                        .map(|reply| Attempt::Answer(identity, reply));
                }
                return Ok(Attempt::Answer(identity, reply));
            }
        } else if let Some(kind) =
            crate::remote::pre_execution(&String::from_utf8_lossy(&stderr), run.exit)
        {
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
            let mut error = Error::new(code, "SSH refused native bootstrap before dispatch")
                .with_recovery(Recovery::preparation());
            if kind == P::Connection {
                error = error.retryable();
            }
            return Err(error);
        }
        Err(Error::new(
            if run.cancelled {
                "REMOTE_COMMAND_CANCELLED"
            } else {
                "REMOTE_EXECUTION_UNKNOWN"
            },
            "verified native answer is unavailable; do not submit a new identity",
        ))
    }
    fn verified(
        &self,
        kind: u32,
        input: &[u8],
        budget: Duration,
        cap: usize,
        files: bool,
    ) -> Result<(Value, Vec<u8>), Error> {
        match self.attempt(kind, input, budget, cap, files)? {
            Attempt::Answer(identity, bytes) => return Ok((identity, bytes)),
            Attempt::Missing if kind == 8 && !files => {
                return Err(Error::new(
                    "HELPER_PREPARATION_FAILED",
                    "matching helper is absent; prepare explicitly",
                )
                .with_recovery(Recovery::preparation()));
            }
            Attempt::Missing => (),
        }
        self.ensure()
            .map_err(|e| e.with_recovery(Recovery::preparation()))?;
        match self.attempt(kind, input, budget, cap, files)? {
            Attempt::Answer(identity, bytes) => Ok((identity, bytes)),
            Attempt::Missing => Err(Error::new(
                "HELPER_PREPARATION_FAILED",
                "prepared helper disappeared before dispatch",
            )
            .with_recovery(Recovery::preparation())),
        }
    }
    pub fn operation(
        &self,
        kind: u32,
        request: Writer,
        wait_ms: u32,
    ) -> Result<(Value, Vec<u8>), Error> {
        self.verified(
            kind,
            &request.frame(kind)?,
            Duration::from_millis(u64::from(wait_ms) + 15000),
            codec::LIMIT + 16,
            false,
        )
    }
    pub(super) fn verified_file(
        &self,
        request: &[u8],
        timeout: Duration,
        cap: usize,
    ) -> Result<Vec<u8>, Error> {
        self.verified(0, request, timeout, cap, true)
            .map(|(_, bytes)| bytes)
            .map_err(|mut error| {
                if error.recovery.submission() != "not_submitted" {
                    error = error.with_mutation(crate::domain::MutationEvidence::Unknown);
                }
                error
            })
    }
}
fn cancelled() -> Error {
    Error::new(
        "REMOTE_COMMAND_CANCELLED",
        "native observation cancelled; remote work may remain active",
    )
}
