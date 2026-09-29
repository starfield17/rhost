#!/usr/bin/env sh
# A single-file entry point: the installation logic below runs in Python.
set -eu
command -v python3 >/dev/null 2>&1 || { echo 'install: Python 3.11+ is required' >&2; exit 2; }
python3 -c 'import sys; sys.exit(sys.version_info < (3, 11))' || { echo 'install: Python 3.11+ is required' >&2; exit 2; }
exec python3 - "$@" <<'PY_INSTALLER'
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request

REPO = "starfield17/rhost"
OS_MAP = {"Darwin": "darwin", "Linux": "linux"}
ARCH_MAP = {"arm64": "arm64", "aarch64": "arm64", "x86_64": "amd64", "amd64": "amd64"}


def request(url):
    return urllib.request.Request(url, headers={"User-Agent": "rhost-installer", "Accept": "application/vnd.github+json"})


def error_detail(error):
    if isinstance(error, urllib.error.HTTPError):
        detail = f"HTTP {error.code}"
        if error.code in (403, 429) and error.headers.get("x-ratelimit-remaining") == "0":
            detail += " (GitHub API rate limit exhausted)"
        return detail
    return str(error)


def latest_version():
    errors = []
    stable_url = f"https://github.com/{REPO}/releases/latest"
    try:
        with urllib.request.urlopen(request(stable_url), timeout=30) as response:
            path = urllib.parse.urlparse(response.geturl()).path
        match = re.fullmatch(rf"/{re.escape(REPO)}/releases/tag/v([^/]+)", path)
        if match:
            return match.group(1)
        errors.append("stable release: no version in redirect")
    except (urllib.error.URLError, OSError) as error:
        errors.append(f"stable release: {error_detail(error)}")
    api_url = f"https://api.github.com/repos/{REPO}/releases?per_page=1"
    try:
        with urllib.request.urlopen(request(api_url), timeout=30) as response:
            releases = json.load(response)
        tag = releases[0]["tag_name"]
        if isinstance(tag, str) and tag.startswith("v"):
            return tag[1:]
        errors.append("release API: no version tag")
    except (urllib.error.URLError, OSError, ValueError, KeyError, IndexError, TypeError) as error:
        errors.append(f"release API: {error_detail(error)}")
    raise RuntimeError("could not determine the newest version (" + "; ".join(errors) +
                       "). Set RHOST_VERSION=<version> to install a known release.")


def download(url, destination, digest=None):
    with urllib.request.urlopen(request(url), timeout=60) as response, destination.open("wb") as output:
        while chunk := response.read(1024 * 1024):
            output.write(chunk)
            if digest is not None:
                digest.update(chunk)


def main():
    os_name = OS_MAP.get(platform.system())
    if os_name is None:
        raise RuntimeError(f"unsupported OS: {platform.system().lower()}")
    arch = ARCH_MAP.get(platform.machine())
    if arch is None:
        raise RuntimeError(f"unsupported architecture: {platform.machine()}")
    version = os.environ.get("RHOST_VERSION", "").removeprefix("v") or latest_version()
    if not re.fullmatch(r"[0-9A-Za-z.+-]+", version):
        raise RuntimeError(f"invalid RHOST_VERSION: {version}")
    asset = f"rhost_{version}_{os_name}_{arch}"
    url = f"https://github.com/{REPO}/releases/download/v{version}/{asset}"
    install_dir = Path(os.environ.get("RHOST_INSTALL_DIR") or Path.home() / ".local/bin").expanduser()
    install_dir.mkdir(parents=True, exist_ok=True)
    print(f"downloading rhost {version} for {os_name}/{arch} ...")
    with tempfile.TemporaryDirectory(prefix=".rhost-install.", dir=install_dir) as temporary:
        binary = Path(temporary) / asset
        digest = hashlib.sha256()
        download(url, binary, digest)
        try:
            with urllib.request.urlopen(request(url + ".sha256"), timeout=30) as response:
                checksum = response.read(4097).decode("ascii")
        except urllib.error.HTTPError as error:
            if error.code == 404:
                raise RuntimeError("no SHA-256 file for this release; refusing to install an unverified binary") from error
            raise
        match = re.fullmatch(rf"([0-9a-fA-F]{{64}})  {re.escape(asset)}\n?", checksum)
        if not match or digest.hexdigest() != match.group(1).lower():
            raise RuntimeError("SHA-256 verification failed; existing installation was preserved")
        binary.chmod(0o755)
        os.replace(binary, install_dir / "rhost")
    print(f"installed {install_dir / 'rhost'}")
    if str(install_dir) not in os.environ.get("PATH", "").split(os.pathsep):
        print(f"note: {install_dir} is not on your PATH, so 'rhost' does not resolve yet")
        print(f'      make it resolve:  export PATH="{install_dir}:$PATH"  (put it in your shell startup file)')
        print("      refresh an open shell:  rehash  (zsh) or  hash -r  (bash)")
        print("      or install into a directory already on your PATH:  RHOST_INSTALL_DIR=<that-directory> ./scripts/install.sh")


try:
    main()
except (OSError, UnicodeError, ValueError, RuntimeError, urllib.error.URLError) as error:
    print(f"install: {error_detail(error)}", file=sys.stderr)
    sys.exit(1)
PY_INSTALLER
