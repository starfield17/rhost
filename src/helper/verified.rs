//! A single SSH channel verifies the loaded image before capability dispatch.
use super::{
    Helper, Writer, artifact, build, codec,
    target::{Fixed, Preparation, Target},
};
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
                    profiles.push((name, "native-c99-o2-v1".into(), String::new()));
                }
            }
        }
        Ok(profiles)
    }
    fn verified_command(
        &self,
        marker: &str,
        files: bool,
        release_probe: bool,
    ) -> Result<String, Error> {
        let explicit = Self::explicit_selection();
        if explicit {
            self.initialize()?;
            if let Some(Preparation::Fixed(fixed)) = self.preparation.borrow().as_ref() {
                return self.fixed_command(marker, files, fixed);
            }
        }
        let base = crate::shell::path_quote(&self.cache_base()?);
        let mut script = if explicit {
            let target = self
                .target
                .get()
                .ok_or_else(|| Error::new("INTERNAL", "target missing"))?;
            format!(
                "printf '%s' {}; rh_os={}; rh_target={}; rh_base={base}; rh_cache=; rh_image=;\n",
                crate::shell::quote(marker),
                crate::shell::quote(&target.os),
                crate::shell::quote(&target.abi()),
            )
        } else {
            format!(
                "printf '%s' {}; rh_os=$(command uname -s) && rh_arch=$(command uname -m) || {{ printf 'RHOST_FAST_DEPENDENCY\\n'; exit 127; }};\ncase \"$rh_os\" in Linux) rh_os=linux;; Darwin) rh_os=darwin;; FreeBSD) rh_os=freebsd;; OpenBSD) rh_os=openbsd;; NetBSD) rh_os=netbsd;; *) printf 'RHOST_FAST_UNSUPPORTED\\n'; exit 126;; esac;\ncase \"$rh_arch\" in aarch64|arm64) rh_arch=aarch64;; x86_64|amd64) rh_arch=x86_64;; *) printf 'RHOST_FAST_UNSUPPORTED\\n'; exit 126;; esac;\nrh_target=\"$rh_os-$rh_arch\"; rh_base={base}; rh_cache=; rh_image=;\n",
                crate::shell::quote(marker)
            )
        };
        if !release_probe {
            if explicit {
                let cache = crate::shell::path_quote(self.cache_path().trim_end_matches('/'));
                let image = crate::shell::quote(&self.expected_image()?.unwrap_or_default());
                script.push_str(&format!(
                    "rh_selected={cache}; if [ -e \"$rh_selected/rhost-helper\" ] || [ -L \"$rh_selected/rhost-helper\" ]; then rh_cache=$rh_selected; rh_image={image}; fi;\n"
                ));
            } else {
                for (target, profile, image) in self.profiles()? {
                    script.push_str(&format!(
                        "if [ -z \"$rh_cache\" ] && [ \"$rh_target\" = {} ] && {{ [ -e \"${{rh_base}}{target}/{profile}/rhost-helper\" ] || [ -L \"${{rh_base}}{target}/{profile}/rhost-helper\" ]; }}; then rh_cache=\"${{rh_base}}{target}/{profile}\"; rh_image={}; fi;\n",
                        crate::shell::quote(&target), crate::shell::quote(&image)
                    ));
                }
            }
        }
        let release = if explicit {
            self.automatic()
                && self.target.get().is_some_and(artifact::supported)
                && self.expected_image()?.is_none()
        } else {
            std::env::var_os("RHOST_HELPER_BIN").is_none()
                && std::env::var_os("RHOST_HELPER_CC").is_none()
        };
        if release {
            // The private cache keeps the digest verified at install time in the
            // profile name. The loaded image must match it before dispatch.
            script.push_str(&format!(
                r#"if [ -z "$rh_cache" ] && [ "$rh_os" = linux ]; then
  rh_prefix="$rh_base$rh_target/"{}
  for rh_candidate in "$rh_prefix"*; do
    rh_digest=${{rh_candidate#"$rh_prefix"}}
    case "$rh_digest" in *[!0123456789abcdef]*) continue;; esac
    [ "${{#rh_digest}}" = 64 ] || continue
    [ -e "$rh_candidate/rhost-helper" ] || [ -L "$rh_candidate/rhost-helper" ] || continue
    rh_cache=$rh_candidate; rh_image=$rh_digest; break
  done
fi
"#,
                crate::shell::quote(artifact::PROFILE_PREFIX)
            ));
        }
        script.push_str(
            "if [ -z \"$rh_cache\" ]; then printf 'RHOST_FAST_MISSING\\n'; exit 127; fi;\n",
        );
        script.push_str(&Self::guard("\"$rh_cache\""));
        script.push_str("rh_helper=\"$rh_cache/rhost-helper\";\n");
        script.push_str(&self.verified_exec(files));
        Ok(format!("sh -c {}", crate::shell::quote(&script)))
    }
    fn fixed_command(&self, marker: &str, files: bool, fixed: &Fixed) -> Result<String, Error> {
        let target = self
            .target
            .get()
            .ok_or_else(|| Error::new("INTERNAL", "target missing"))?;
        let mut script = format!(
            "printf '%s' {}; rh_helper={}; rh_target={}; rh_image={};\n",
            crate::shell::quote(marker),
            crate::shell::path_quote(&fixed.path),
            crate::shell::quote(&target.abi()),
            crate::shell::quote(fixed.pin.as_ref().map(|p| p.as_str()).unwrap_or("")),
        );
        script.push_str(Self::ancestry_guard());
        script.push_str(
            r#"rh_refuse() { printf 'RHOST_FAST_FIXED_REFUSED\n'; exit 126; }
case "$rh_helper" in /*) ;; *) rh_refuse;; esac
rh_parent=${rh_helper%/*}
rh_safe_ancestry "${rh_parent:-/}" || rh_refuse
[ ! -L "$rh_helper" ] && [ -f "$rh_helper" ] && [ -x "$rh_helper" ] || rh_refuse
rh_info=$(stat -c '%u %a %s' "$rh_helper" 2>/dev/null) || rh_info=$(stat -f '%u %Lp %z' "$rh_helper" 2>/dev/null) || rh_refuse
set -- $rh_info; [ "$#" = 3 ] || rh_refuse
[ "$1" = 0 ] || [ "$1" = "$rh_uid" ] || rh_refuse
case "$2" in ''|*[!0-7]*) rh_refuse;; esac
[ $((0$2 & 06022)) = 0 ] || rh_refuse
[ "$3" -gt 0 ] && [ "$3" -le 8388608 ] || rh_refuse
"#,
        );
        if fixed.pin.is_some() {
            // Check a pin before executing anything at the configured path.
            // The helper also checks the loaded image before dispatch.
            script.push_str(
                r#"rh_hash=$(command sha256sum 2>/dev/null < "$rh_helper") ||
  rh_hash=$(command sha256 -q 2>/dev/null < "$rh_helper") ||
  rh_hash=$(command shasum -a 256 2>/dev/null < "$rh_helper") ||
  { printf 'RHOST_FAST_DEPENDENCY\n'; exit 127; }
[ "${rh_hash%% *}" = "$rh_image" ] || rh_refuse
"#,
            );
        }
        script.push_str(&self.verified_exec(files));
        Ok(format!("sh -c {}", crate::shell::quote(&script)))
    }
    fn verified_exec(&self, files: bool) -> String {
        let state = crate::shell::path_quote(
            &std::env::var("RHOST_REMOTE_STATE")
                .unwrap_or_else(|_| "~/.local/state/rhost/v5".into()),
        );
        format!(
            "exec env RHOST_REMOTE_STATE={state} RHOST_EXPECT_BOOT_ID={} \"$rh_helper\" {} {} \"$rh_image\" \"$rh_target\"",
            crate::shell::quote(self.boot_guard.map(|b| b.as_str()).unwrap_or("")),
            if files {
                "--verified-files"
            } else {
                "--verified-run"
            },
            crate::shell::quote(&super::source_digest())
        )
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
            .verified_command(&marker, files, kind == 1 && !files)
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
                (
                    b"RHOST_FAST_FIXED_REFUSED\n".as_slice(),
                    "HELPER_INCOMPATIBLE",
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
                let pin = if Self::explicit_selection() {
                    self.expected_image()?
                } else {
                    None
                };
                let identity = self.decode_probe(&handshake, pin.as_deref())?;
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
    pub(super) fn probe_fixed(&self) -> Result<Value, Error> {
        let request = Writer::default().frame(1)?;
        match self.attempt(1, &request, Duration::from_secs(15), 65536, false)? {
            Attempt::Answer(identity, _) => Ok(identity),
            Attempt::Missing => Err(Error::new("HELPER_INCOMPATIBLE", "fixed helper is absent")
                .with_recovery(Recovery::preparation())),
        }
    }
    pub(super) fn probe_release(&self) -> Result<Option<Value>, Error> {
        if !self.target.get().is_some_and(artifact::supported) {
            return Ok(None);
        }
        let request = Writer::default().frame(1)?;
        match self.attempt(1, &request, Duration::from_secs(15), 65536, false) {
            Ok(Attempt::Answer(identity, _)) => Ok(Some(identity)),
            Ok(Attempt::Missing) => Ok(None),
            Err(error) if super::prepare::recoverable(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }
    fn verified(
        &self,
        kind: u32,
        input: &[u8],
        budget: Duration,
        cap: usize,
        files: bool,
    ) -> Result<(Value, Vec<u8>), Error> {
        match self.attempt(kind, input, budget, cap, files) {
            Ok(Attempt::Answer(identity, bytes)) => return Ok((identity, bytes)),
            Ok(Attempt::Missing) if self.fixed() || (kind == 8 && !files) => {
                return Err(Error::new(
                    "HELPER_PREPARATION_FAILED",
                    "matching helper is absent; prepare explicitly",
                )
                .with_recovery(Recovery::preparation()));
            }
            Ok(Attempt::Missing) => (),
            // Explicit preparation can repair a known incompatible image only
            // after a complete refusal proves no capability was dispatched.
            Err(error)
                if kind == 7
                    && !files
                    && !self.fixed()
                    && error.code == "HELPER_INCOMPATIBLE"
                    && error.recovery.submission() == "not_submitted" => {}
            Err(error) => return Err(error),
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
