//! One-shot native helper transport over the existing OpenSSH client.
mod artifact;
mod build;
pub mod codec;
mod prepare;
mod sources;
mod target;
use crate::remote::Error;
use crate::transport::{Client, Recorder, Run, StdinSource, Streams};
pub use codec::{Reader, Writer};
use std::time::Duration;
pub fn source_digest() -> String {
    sources::digest()
}

pub struct Helper<'a> {
    client: &'a Client,
    host: &'a str,
    cache: String,
    target: std::sync::OnceLock<target::Target>,
    selected_cache: std::cell::RefCell<Option<String>>,
    preparation: std::cell::RefCell<Option<target::Preparation>>,
    cancelled: &'a dyn Fn() -> bool,
    boot_guard: Option<&'a crate::domain::BootId>,
}
impl<'a> Helper<'a> {
    pub fn new(client: &'a Client, host: &'a str) -> Self {
        Self {
            client,
            host,
            cache: format!("~/.cache/rhost/helper/p1/{}/", source_digest()),
            target: std::sync::OnceLock::new(),
            selected_cache: std::cell::RefCell::new(None),
            preparation: std::cell::RefCell::new(None),
            cancelled: &never,
            boot_guard: None,
        }
    }
    pub fn with_cancellation(mut self, cancelled: &'a dyn Fn() -> bool) -> Self {
        self.cancelled = cancelled;
        self
    }
    pub fn with_boot_guard(mut self, guard: Option<&'a crate::domain::BootId>) -> Self {
        self.boot_guard = guard;
        self
    }
    fn transport(
        &self,
        command: &str,
        input: &[u8],
        budget: Duration,
        cap: usize,
    ) -> Result<(Run, Vec<u8>, Vec<u8>), Error> {
        if (self.cancelled)() {
            return Err(Error::new(
                "REMOTE_COMMAND_CANCELLED",
                "local observation cancelled",
            ));
        }
        let mut stdout = Recorder::new(cap);
        let mut stderr = Recorder::new(cap);
        let run = self
            .client
            .run_streams(
                &Streams {
                    target: self.host,
                    remote_command: command,
                    timeout: Some(budget),
                    stdin: StdinSource::Payload(input),
                    fresh: false,
                    cancelled: self.cancelled,
                },
                &mut stdout,
                &mut stderr,
            )
            .map_err(|e| Error::new("CONFIG_INVALID", e.to_string()))?;
        Ok((run, stdout.body(), stderr.body()))
    }
    fn command(&self, marker: &str) -> String {
        self.command_mode(marker, "")
    }
    fn command_mode(&self, marker: &str, mode: &str) -> String {
        let state = std::env::var("RHOST_REMOTE_STATE")
            .unwrap_or_else(|_| "~/.local/state/rhost/v5".into());
        let executable = format!("{}rhost-helper", self.cache_path());
        let script = format!(
            "printf '%s' {}; if [ ! -e {executable_path} ] && [ ! -L {executable_path} ]; then printf 'RHOST_HELPER_ABSENT\\n'; exit 127; fi; {} exec env RHOST_REMOTE_STATE={} RHOST_EXPECT_BOOT_ID={} {} {mode}",
            crate::shell::quote(marker),
            self.cache_guard(),
            crate::shell::path_quote(&state),
            crate::shell::quote(
                self.boot_guard
                    .map(crate::domain::BootId::as_str)
                    .unwrap_or("")
            ),
            crate::shell::path_quote(&executable),
            executable_path = crate::shell::path_quote(&executable)
        );
        format!("sh -c {}", crate::shell::quote(&script))
    }
    fn cache_guard(&self) -> String {
        let cache = crate::shell::path_quote(self.cache_path().trim_end_matches('/'));
        // A same-account writer is trusted. Other accounts cannot change the
        // checked private leaf; reject symlinks and writable ancestry on reuse.
        format!(
            r#"set -f; rh_cache={cache}; rh_p=; rh_uid=$(id -u) || exit 126;
rh_oldifs=$IFS; IFS=/; set -- $rh_cache; IFS=$rh_oldifs;
for rh_component do
  [ -n "$rh_component" ] || continue;
  rh_p="$rh_p/$rh_component";
  [ ! -L "$rh_p" ] && [ -d "$rh_p" ] || exit 126;
  rh_info=$(stat -c '%u %a' "$rh_p" 2>/dev/null) || rh_info=$(stat -f '%u %Lp' "$rh_p" 2>/dev/null) || exit 126;
  set -- $rh_info; [ "$#" = 2 ] || exit 126;
  [ "$1" = 0 ] || [ "$1" = "$rh_uid" ] || exit 126;
  case "$2" in ''|*[!0-7]*) exit 126;; esac;
  [ $((0$2 & 022)) = 0 ] || [ $((0$2 & 01000)) != 0 ] || exit 126;
done;
rh_info=$(stat -c '%u %a' "$rh_cache" 2>/dev/null) || rh_info=$(stat -f '%u %Lp' "$rh_cache" 2>/dev/null) || exit 126;
set -- $rh_info; [ "$1" = "$rh_uid" ] && [ $((0$2 & 077)) = 0 ] || exit 126;
[ ! -L "$rh_cache/rhost-helper" ] && [ -f "$rh_cache/rhost-helper" ] || exit 126;
rh_info=$(stat -c '%u %a' "$rh_cache/rhost-helper" 2>/dev/null) || rh_info=$(stat -f '%u %Lp' "$rh_cache/rhost-helper" 2>/dev/null) || exit 126;
set -- $rh_info; [ "$1" = "$rh_uid" ] && [ $((0$2 & 077)) = 0 ] || exit 126;
"#
        )
    }
    fn raw(&self, kind: u32, request: Vec<u8>, budget: Duration) -> Result<Vec<u8>, Error> {
        self.initialize()?;
        let nonce = crate::random::hex(16).map_err(|e| Error::new("INTERNAL", e.to_string()))?;
        let marker = format!("RHOST_FRAME_{kind}_{nonce}\n");
        let (run, stdout, _) = self.transport(
            &self.command(&marker),
            &request,
            budget,
            codec::LIMIT + 16 + 65536,
        )?;
        let start = stdout
            .windows(marker.len())
            .position(|w| w == marker.as_bytes())
            .map(|n| n + marker.len());
        if let Some(at) = start {
            let bytes = &stdout[at..];
            if kind == 1 && bytes == b"RHOST_HELPER_ABSENT\n" {
                return Err(Error::new(
                    "HELPER_PREPARATION_FAILED",
                    "matching native helper cache is absent",
                ));
            }
            if bytes.starts_with(b"RH01") {
                return codec::answer(bytes, kind);
            }
        }
        if run.cancelled {
            return Err(Error::new(
                "REMOTE_COMMAND_CANCELLED",
                "local observation cancelled; the Run may still be running",
            ));
        }
        Err(Error::new(
            "REMOTE_EXECUTION_UNKNOWN",
            "no complete native helper answer; inspect or resubmit the same request key",
        ))
    }
    pub(super) fn probe_current(&self) -> Result<serde_json::Value, Error> {
        let bytes = self.raw(1, Writer::default().frame(1)?, Duration::from_secs(15))?;
        let mut r = Reader::new(&bytes);
        let digest = r.text()?;
        let os = r.text()?;
        let arch = r.text()?;
        let capabilities = r.int()?;
        let boot = r.text()?;
        if !boot.is_empty() {
            crate::domain::BootId::new(boot.clone())
                .map_err(|_| codec::protocol("invalid remote boot identity"))?;
        }
        let image = r.text()?;
        crate::domain::Sha256::new(image.clone())
            .map_err(|_| codec::protocol("invalid helper image digest"))?;
        if let Some(expected) = self.expected_image()?
            && expected != image
        {
            return Err(Error::new(
                "HELPER_INCOMPATIBLE",
                "cached helper bytes do not match selected image",
            ));
        }
        for bound in [8388608, 12582912, 8388608, 67108864, 1048576] {
            if r.int()? != bound {
                return Err(codec::protocol("native limits do not match"));
            }
        }
        r.done()?;
        if let Some(t) = self.target.get()
            && (t.os != os || t.arch != arch)
        {
            return Err(Error::new(
                "HELPER_INCOMPATIBLE",
                "helper target does not match remote observation",
            ));
        }
        if digest != source_digest() {
            return Err(Error::new(
                "HELPER_INCOMPATIBLE",
                "cached helper source identity does not match",
            ));
        }
        if capabilities & 7 != 7 {
            return Err(Error::new(
                "HELPER_INCOMPATIBLE",
                "native operation guarantees are unavailable",
            ));
        }
        if let Some(expected) = self.boot_guard {
            if boot.is_empty() {
                return Err(Error::new(
                    "CONTEXT_UNAVAILABLE",
                    "remote boot identity is unavailable",
                ));
            }
            if expected.as_str() != boot {
                return Err(Error::new(
                    "CONTEXT_CHANGED",
                    "remote boot identity changed",
                ));
            }
        }
        Ok(
            serde_json::json!({"protocol":1,"source_digest":digest,"os":os,"arch":arch,"capabilities":capabilities,"binary_sha256":image,"boot_id":if boot.is_empty(){None}else{Some(boot)},"limits":{"run_frame_bytes":8388608,"file_request_bytes":12582912,"file_edit_bytes":8388608,"run_stream_bytes":67108864,"run_page_bytes":1048576}}),
        )
    }
    pub fn call(&self, kind: u32, request: Writer, wait_ms: u32) -> Result<Vec<u8>, Error> {
        self.raw(
            kind,
            request.frame(kind)?,
            Duration::from_millis(u64::from(wait_ms) + 15000),
        )
    }
    pub fn file_call(
        &self,
        request: &[u8],
        timeout: Duration,
        cap: usize,
    ) -> Result<Vec<u8>, Error> {
        self.ensure()?;
        let marker = format!(
            "RHOST_FILE_{}\n",
            crate::random::hex(16).map_err(|e| Error::new("INTERNAL", e.to_string()))?
        );
        let (run, stdout, _) = self
            .transport(
                &self.command_mode(&marker, "--files-json"),
                request,
                timeout,
                cap.saturating_add(65536),
            )
            .map_err(|e| e.with_mutation(crate::domain::MutationEvidence::Unknown))?;
        if let Some(at) = stdout
            .windows(marker.len())
            .position(|s| s == marker.as_bytes())
        {
            let answer = &stdout[at + marker.len()..];
            if serde_json::from_slice::<serde_json::Value>(answer).is_ok() {
                return Ok(answer.to_vec());
            }
        }
        if run.cancelled {
            return Err(Error::new(
                "REMOTE_COMMAND_CANCELLED",
                "file observation cancelled; mutation outcome is unknown",
            )
            .with_mutation(crate::domain::MutationEvidence::Unknown));
        }
        Err(Error::new(
            "REMOTE_EXECUTION_UNKNOWN",
            "no complete native file helper answer; mutation outcome is unknown",
        )
        .with_mutation(crate::domain::MutationEvidence::Unknown))
    }
}
fn never() -> bool {
    false
}

#[cfg(test)]
mod tests;
