//! An explicitly configured LLVM/musl build; no toolchain or package installation.
use super::{artifact::Binary, sources, target::Target};
use crate::{
    remote::Error,
    transport::{self, Recorder, Spec, StdinSource},
};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
pub(super) struct Configuration {
    pub identity: String,
    compiler: PathBuf,
    sysroot: PathBuf,
    builtins: PathBuf,
    linker: PathBuf,
    target: String,
}
fn config(message: impl Into<String>) -> Error {
    Error::new("CONFIG_INVALID", message)
}
fn required(name: &str) -> Result<PathBuf, Error> {
    std::env::var_os(name)
        .ok_or_else(|| config(format!("set {name} for a configured helper build")))
        // Multi-call linkers choose their mode from argv[0] (ld.lld vs lld).
        // Preserve that spelling while hashing the file it resolves to.
        .and_then(|p| std::path::absolute(p).map_err(|e| config(e.to_string())))
}
fn hash_file(
    path: &Path,
    hash: &mut Sha256,
    total: &mut u64,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), Error> {
    if !std::fs::metadata(path)
        .map_err(|e| config(e.to_string()))?
        .is_file()
    {
        return Err(config(
            "configured compiler and library inputs must be regular files",
        ));
    }
    let mut f = std::fs::File::open(path).map_err(|e| config(e.to_string()))?;
    let mut block = [0u8; 65536];
    loop {
        if cancelled() {
            return Err(Error::new(
                "REMOTE_COMMAND_CANCELLED",
                "build-input observation cancelled",
            ));
        }
        let n = f.read(&mut block).map_err(|e| config(e.to_string()))?;
        if n == 0 {
            break;
        }
        *total += n as u64;
        if *total > 1024 * 1024 * 1024 {
            return Err(config("configured build inputs exceed 1 GiB"));
        }
        hash.update(&block[..n]);
    }
    Ok(())
}
fn tree(
    path: &Path,
    base: &Path,
    hash: &mut Sha256,
    total: &mut u64,
    count: &mut usize,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), Error> {
    let mut entries = std::fs::read_dir(path)
        .map_err(|e| config(e.to_string()))?
        .take(50001usize.saturating_sub(*count))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| config(e.to_string()))?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        *count += 1;
        if *count > 50000 {
            return Err(config("configured sysroot exceeds 50000 entries"));
        }
        let p = entry.path();
        let info = std::fs::symlink_metadata(&p).map_err(|e| config(e.to_string()))?;
        if info.file_type().is_symlink() {
            return Err(config(
                "configured sysroot must contain regular files and directories",
            ));
        }
        hash.update(
            p.strip_prefix(base)
                .map_err(|_| config("invalid sysroot path"))?
                .as_os_str()
                .as_encoded_bytes(),
        );
        hash.update([0]);
        if info.is_dir() {
            hash.update(b"dir\0");
            tree(&p, base, hash, total, count, cancelled)?;
        } else if info.is_file() {
            hash.update(b"file\0");
            hash.update(info.len().to_be_bytes());
            hash_file(&p, hash, total, cancelled)?;
        } else {
            return Err(config("unsupported sysroot entry"));
        }
    }
    Ok(())
}
impl Configuration {
    pub fn load(target: &Target, cancelled: &dyn Fn() -> bool) -> Result<Self, Error> {
        if target.os != "linux" {
            return Err(config("configured LLVM/musl builds target Linux"));
        }
        let compiler = required("RHOST_HELPER_CC")?;
        let sysroot = required("RHOST_HELPER_SYSROOT")?;
        let builtins = required("RHOST_HELPER_BUILTINS")?;
        let linker = required("RHOST_HELPER_LD")?;
        let mut hash = Sha256::new();
        hash.update(b"llvm-musl-c99-os-static-v1\0");
        hash.update(super::source_digest());
        hash.update(target.arch.as_bytes());
        let mut total = 0;
        for (role, path) in [
            ("compiler", &compiler),
            ("builtins", &builtins),
            ("linker", &linker),
        ] {
            hash.update(role);
            hash.update([0]);
            hash_file(path, &mut hash, &mut total, cancelled)?;
            hash.update([0]);
        }
        tree(&sysroot, &sysroot, &mut hash, &mut total, &mut 0, cancelled)?;
        let identity = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        Ok(Self {
            identity,
            compiler,
            sysroot,
            builtins,
            linker,
            target: format!("{}-linux-musl", target.arch),
        })
    }
    pub fn compile(&self, cancelled: &dyn Fn() -> bool) -> Result<Binary, Error> {
        let temp = std::env::temp_dir().join(format!(
            "rhost-helper-build-{}",
            crate::random::hex(16).map_err(|e| config(e.to_string()))?
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&temp)
            .map_err(|e| config(e.to_string()))?;
        let scratch = Scratch(temp);
        for (name, source) in sources::SOURCES {
            std::fs::write(scratch.0.join(name), source).map_err(|e| config(e.to_string()))?;
        }
        let output = scratch.0.join("helper");
        let lib = self.sysroot.join("usr/lib");
        let mut args = vec![
            "-i".into(),
            "LC_ALL=C".into(),
            "SOURCE_DATE_EPOCH=0".into(),
            self.compiler.to_string_lossy().into_owned(),
            "--no-default-config".into(),
            format!("--target={}", self.target),
            format!("--sysroot={}", self.sysroot.display()),
            "-nostdinc".into(),
            "-isystem".into(),
            self.sysroot
                .join("usr/include")
                .to_string_lossy()
                .into_owned(),
            "-std=c99".into(),
            "-Os".into(),
            "-Wall".into(),
            "-Wextra".into(),
            "-Werror".into(),
            "-static".into(),
            "-nostdlib".into(),
            format!("--ld-path={}", self.linker.display()),
            format!("-ffile-prefix-map={}=.", scratch.0.display()),
            format!("-DRHOST_SOURCE_DIGEST=\"{}\"", super::source_digest()),
            lib.join("crt1.o").to_string_lossy().into_owned(),
            lib.join("crti.o").to_string_lossy().into_owned(),
        ];
        for (name, _) in sources::SOURCES
            .iter()
            .filter(|(name, _)| name.ends_with(".c"))
        {
            args.push(scratch.0.join(name).to_string_lossy().into_owned());
        }
        args.extend([
            "-Wl,--start-group".into(),
            lib.join("libc.a").to_string_lossy().into_owned(),
            self.builtins.to_string_lossy().into_owned(),
            "-Wl,--end-group".into(),
            lib.join("crtn.o").to_string_lossy().into_owned(),
            "-o".into(),
            output.to_string_lossy().into_owned(),
        ]);
        let mut stdout = Recorder::new(4096);
        let mut stderr = Recorder::new(65536);
        let run = transport::run(
            &Spec {
                program: "env",
                args: &args,
                stdin: StdinSource::Closed,
                deadline: Some(Instant::now() + Duration::from_secs(120)),
                cancelled,
                group: true,
            },
            &mut stdout,
            &mut stderr,
        );
        if run.cancelled {
            return Err(Error::new(
                "REMOTE_COMMAND_CANCELLED",
                "local helper compilation cancelled",
            ));
        }
        if run.exit != Some(0) || run.failure.is_some() {
            return Err(Error::new(
                "HELPER_PREPARATION_FAILED",
                format!(
                    "configured compiler failed: {}",
                    String::from_utf8_lossy(&stderr.body())
                ),
            ));
        }
        Binary::read(output.as_os_str())
    }
}
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
