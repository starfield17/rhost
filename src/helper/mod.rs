//! One-shot native helper transport over the existing OpenSSH client.
pub mod codec;
mod sources;
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
    cancelled: &'a dyn Fn() -> bool,
    boot_guard: Option<&'a crate::domain::BootId>,
}
impl<'a> Helper<'a> {
    pub fn new(client: &'a Client, host: &'a str) -> Self {
        Self {
            client,
            host,
            cache: format!("~/.cache/rhost/helper/{}/", source_digest()),
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
        let executable = format!("{}rhost-helper", self.cache);
        let script = format!(
            "printf '%s' {}; {} exec env RHOST_REMOTE_STATE={} RHOST_EXPECT_BOOT_ID={} {} {mode}",
            crate::shell::quote(marker),
            self.cache_guard(),
            crate::shell::path_quote(&state),
            crate::shell::quote(
                self.boot_guard
                    .map(crate::domain::BootId::as_str)
                    .unwrap_or("")
            ),
            crate::shell::path_quote(&executable)
        );
        format!("sh -c {}", crate::shell::quote(&script))
    }
    fn cache_guard(&self) -> String {
        let cache = crate::shell::path_quote(self.cache.trim_end_matches('/'));
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
        let nonce = crate::random::hex(16).map_err(|e| Error::new("INTERNAL", e.to_string()))?;
        let marker = format!("RHOST_FRAME_{nonce}\n");
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
    pub fn probe(&self) -> Result<serde_json::Value, Error> {
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
        for bound in [8388608, 12582912, 8388608, 67108864, 1048576] {
            if r.int()? != bound {
                return Err(codec::protocol("native limits do not match"));
            }
        }
        r.done()?;
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
            serde_json::json!({"protocol":1,"source_digest":digest,"os":os,"arch":arch,"capabilities":capabilities,"boot_id":if boot.is_empty(){None}else{Some(boot)},"limits":{"run_frame_bytes":8388608,"file_request_bytes":12582912,"file_edit_bytes":8388608,"run_stream_bytes":67108864,"run_page_bytes":1048576}}),
        )
    }
    pub fn install(&self) -> Result<serde_json::Value, Error> {
        if std::env::var_os("RHOST_NO_UPLOAD").is_some() {
            return Err(Error::new(
                "HELPER_PREPARATION_FAILED",
                "helper upload is disabled",
            ));
        }
        let marker = format!(
            "RHOST_PREPARED_{}",
            crate::random::hex(16).map_err(|e| Error::new("INTERNAL", e.to_string()))?
        );
        let (command, input) = if let Some(path) = std::env::var_os("RHOST_HELPER_BIN") {
            use std::io::Read;
            let info = std::fs::symlink_metadata(&path)
                .map_err(|e| Error::new("CONFIG_INVALID", e.to_string()))?;
            if !info.is_file() || info.file_type().is_symlink() {
                return Err(Error::new(
                    "CONFIG_INVALID",
                    "configured helper must be a regular local file",
                ));
            }
            let mut bytes = Vec::new();
            std::fs::File::open(&path)
                .map_err(|e| Error::new("CONFIG_INVALID", e.to_string()))?
                .take((codec::LIMIT + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|e| Error::new("CONFIG_INVALID", e.to_string()))?;
            if bytes.is_empty() || bytes.len() > codec::LIMIT {
                return Err(Error::new(
                    "CONFIG_INVALID",
                    "configured helper exceeds binary size limit",
                ));
            }
            let script = sources::binary_script(&self.cache, &marker);
            (format!("sh -c {}", crate::shell::quote(&script)), bytes)
        } else {
            if std::env::var_os("RHOST_NO_SOURCE_BUILD").is_some() {
                return Err(Error::new(
                    "HELPER_PREPARATION_FAILED",
                    "source builds are disabled; provide a verified helper artifact",
                ));
            }
            (
                "sh -s".into(),
                sources::script(&self.cache, &marker).into_bytes(),
            )
        };
        let (run, stdout, stderr) =
            self.transport(&command, &input, Duration::from_secs(90), 65536)?;
        if run.cancelled {
            return Err(Error::new(
                "REMOTE_COMMAND_CANCELLED",
                "helper preparation observation cancelled",
            ));
        }
        if run.exit != Some(0)
            || !stdout
                .split(|b| *b == b'\n')
                .any(|line| line == marker.as_bytes())
        {
            let diagnostic = String::from_utf8_lossy(&stderr);
            return Err(Error::new(
                "HELPER_PREPARATION_FAILED",
                format!(
                    "helper source preparation failed: {}",
                    diagnostic.chars().take(4096).collect::<String>()
                ),
            ));
        }
        self.probe()
    }
    pub fn ensure(&self) -> Result<serde_json::Value, Error> {
        match self.probe() {
            Ok(value) => Ok(value),
            Err(error)
                if matches!(
                    error.code,
                    "REMOTE_COMMAND_CANCELLED" | "CONTEXT_CHANGED" | "CONTEXT_UNAVAILABLE"
                ) =>
            {
                Err(error)
            }
            Err(_) => self.install(),
        }
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
