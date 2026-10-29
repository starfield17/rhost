#!/usr/bin/env python3
"""Local-only fixed-argv helper builds; write path-free provenance beside bytes."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parent.parent


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def required(name):
    path = Path(os.path.abspath(os.environ[name]))
    if not path.is_file():
        raise ValueError(f"{name} must name a regular file")
    return path


def main():
    if os.environ.get("CI"):
        raise ValueError("helper compilation is local-only; CI consumes verified bytes")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("target", choices=["linux-x86_64", "linux-aarch64", "darwin-aarch64"])
    parser.add_argument("--output", type=Path, default=ROOT / "target/helper-release")
    args = parser.parse_args()
    target = args.target
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["version"]
    source = subprocess.check_output(["sh", "scripts/helper-source-digest.sh"], cwd=ROOT, text=True).strip()
    compiler = required("RHOST_HELPER_CC")
    stripper = required("RHOST_HELPER_STRIP")
    config = {"profile": "c99-os-v1", "target": target, "source_digest": source,
              "compiler_sha256": digest(compiler), "strip_sha256": digest(stripper),
              "compiler_version": subprocess.check_output([compiler, "--version"], text=True).splitlines()[0]}
    args.output.mkdir(parents=True, exist_ok=True)
    asset = f"rhost-helper_{version}_{target}"
    output = args.output / asset
    flags = ["-std=c99", "-Os", "-Wall", "-Wextra", "-Werror"]
    sources = sorted((ROOT / "helper").glob("*.c"))
    if target.startswith("linux-"):
        arch = target.removeprefix("linux-")
        sysroot = Path(os.environ["RHOST_HELPER_SYSROOT"]).resolve(strict=True)
        linker, builtins = required("RHOST_HELPER_LD"), required("RHOST_HELPER_BUILTINS")
        tree = hashlib.sha256()
        for item in sorted(sysroot.rglob("*")):
            if item.is_symlink() or not (item.is_file() or item.is_dir()):
                raise ValueError("sysroot must contain only regular files/directories")
            tree.update(str(item.relative_to(sysroot)).encode() + b"\0")
            tree.update((digest(item) if item.is_file() else "directory").encode() + b"\0")
        config.update(linker_sha256=digest(linker), builtins_sha256=digest(builtins),
                      sysroot_sha256=tree.hexdigest(), abi="static-musl")
        flags += ["--no-default-config", f"--target={arch}-linux-musl", f"--sysroot={sysroot}",
                  "-nostdinc", "-isystem", str(sysroot / "usr/include"), "-static", "-nostdlib",
                  f"--ld-path={linker}"]
        lib = sysroot / "usr/lib"
        inputs = [lib / "crt1.o", lib / "crti.o", *sources, "-Wl,--start-group",
                  lib / "libc.a", builtins, "-Wl,--end-group", lib / "crtn.o"]
    else:
        config.update(abi="native-darwin", deployment_target="13.0")
        flags += ["-arch", "arm64", "-mmacosx-version-min=13.0"]
        inputs = sources
    # Neither a compiler's default config nor ambient CFLAGS enters this argv.
    command = [compiler, *flags, f'-DRHOST_SOURCE_DIGEST="{source}"',
               f"-ffile-prefix-map={ROOT}=.", *inputs, "-o", output]
    subprocess.run(command, cwd=ROOT, check=True)
    subprocess.run([stripper, output], check=True)
    output.chmod(0o700)
    encoded = json.dumps(config, sort_keys=True, separators=(",", ":")).encode()
    record = {"target": target, "asset": asset, "sha256": digest(output), "size": output.stat().st_size,
              "role": "runtime" if target.startswith("linux-") else "fixture",
              "build_config": hashlib.sha256(encoded).hexdigest()}
    metadata = {"source_digest": source, "source_commit": subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(), "artifact": record, "configuration": config}
    (args.output / f"{asset}.build.json").write_text(json.dumps(metadata, indent=2) + "\n")
    print(json.dumps(record))


if __name__ == "__main__":
    main()
