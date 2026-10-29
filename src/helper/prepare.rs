//! Cache observation precedes explicitly bounded preparation attempts.
use super::{
    Helper,
    artifact::{self, Binary},
    sources,
    target::Preparation,
};
use crate::remote::Error;
use std::time::Duration;
fn recoverable(error: &Error) -> bool {
    matches!(
        error.code,
        "HELPER_PREPARATION_FAILED" | "HELPER_INCOMPATIBLE"
    )
}
impl Helper<'_> {
    pub fn probe(&self) -> Result<serde_json::Value, Error> {
        self.initialize()?;
        match self.probe_current() {
            Ok(value) => Ok(value),
            Err(error) if self.automatic() && recoverable(&error) => {
                let target = self
                    .target
                    .get()
                    .ok_or_else(|| Error::new("INTERNAL", "target observation missing"))?;
                if let Some((_, pinned)) = artifact::pinned(target)? {
                    self.select_profile(&format!("release-{}", pinned.sha256))?;
                    match self.probe_current() {
                        Ok(value) => return Ok(value),
                        Err(e) if recoverable(&e) => (),
                        Err(e) => return Err(e),
                    }
                }
                Err(error)
            }
            Err(error) => Err(error),
        }
    }
    fn preparation_step(&self, command: &str, input: &[u8], marker: &str) -> Result<(), Error> {
        let (run, stdout, stderr) =
            self.transport(command, input, Duration::from_secs(90), 65536)?;
        if run.cancelled {
            return Err(Error::new(
                "REMOTE_COMMAND_CANCELLED",
                "helper preparation observation cancelled",
            ));
        }
        if run.exit == Some(255)
            && !stdout
                .split(|b| *b == b'\n')
                .any(|s| s == marker.as_bytes())
        {
            return Err(Error::new(
                "REMOTE_EXECUTION_UNKNOWN",
                "helper preparation did not return evidence; the requested operation was not submitted",
            ));
        }
        if run.exit != Some(0)
            || !stdout
                .split(|b| *b == b'\n')
                .any(|s| s == marker.as_bytes())
        {
            return Err(Error::new(
                "HELPER_PREPARATION_FAILED",
                format!(
                    "helper preparation failed: {}",
                    String::from_utf8_lossy(&stderr)
                        .chars()
                        .take(4096)
                        .collect::<String>()
                ),
            ));
        }
        Ok(())
    }
    fn upload(&self, binary: &Binary, marker: &str) -> Result<(), Error> {
        let script = sources::binary_script(&self.cache_path(), marker, &binary.sha256);
        self.preparation_step(
            &format!("sh -c {}", crate::shell::quote(&script)),
            &binary.bytes,
            marker,
        )
    }
    pub fn install(&self) -> Result<serde_json::Value, Error> {
        self.initialize()?;
        if std::env::var_os("RHOST_NO_UPLOAD").is_some() {
            return Err(Error::new(
                "HELPER_PREPARATION_FAILED",
                "helper upload is disabled; only matching cache reuse is allowed",
            ));
        }
        let marker = format!(
            "RHOST_PREPARED_{}",
            crate::random::hex(16).map_err(|e| Error::new("INTERNAL", e.to_string()))?
        );
        match self
            .preparation
            .borrow()
            .as_ref()
            .ok_or_else(|| Error::new("INTERNAL", "preparation selection missing"))?
        {
            Preparation::Supplied(binary) => self.upload(binary, &marker)?,
            Preparation::Local(configuration) => {
                let binary = configuration.compile(self.cancelled)?;
                self.upload(&binary, &marker)?;
            }
            Preparation::Automatic => {
                self.select_profile("native-c99-o2-v1")?;
                let source = if std::env::var_os("RHOST_NO_SOURCE_BUILD").is_some() {
                    Err(Error::new(
                        "HELPER_PREPARATION_FAILED",
                        "source builds are disabled",
                    ))
                } else {
                    self.preparation_step(
                        "sh -s",
                        sources::script(&self.cache_path(), &marker).as_bytes(),
                        &marker,
                    )
                };
                if let Err(error) = source {
                    if error.code != "HELPER_PREPARATION_FAILED" {
                        return Err(error);
                    }
                    let target = self
                        .target
                        .get()
                        .ok_or_else(|| Error::new("INTERNAL", "target missing"))?;
                    let Some((release, pinned)) = artifact::pinned(target)? else {
                        return Err(error);
                    };
                    let binary = artifact::fetch(&release, &pinned, self.cancelled)?;
                    self.select_profile(&format!("release-{}", pinned.sha256))?;
                    self.upload(&binary, &marker)?;
                }
            }
        }
        self.probe_current()
    }
    pub fn ensure(&self) -> Result<serde_json::Value, Error> {
        match self.probe() {
            Ok(value) => Ok(value),
            Err(error) if recoverable(&error) => self.install(),
            Err(error) => Err(error),
        }
    }
}
