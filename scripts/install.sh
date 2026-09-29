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
import shutil
import subprocess
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


def active_standalone_rhost():
    found = shutil.which("rhost")
    if not found:
        return None
    path = Path(found).absolute()
    if path.is_symlink() or not path.is_file():
        return None
    if not os.access(path, os.W_OK) or not os.access(path.parent, os.W_OK):
        return None
    try:
        result = subprocess.run([str(path), "version", "--json"], capture_output=True,
                                text=True, timeout=5, check=False)
        answer = json.loads(result.stdout)
    except (OSError, UnicodeError, ValueError, subprocess.TimeoutExpired):
        return None
    if (result.returncode == 0 and isinstance(answer, dict) and
            answer.get("schema_version") == 2 and
            answer.get("operation") == "version" and answer.get("ok") is True and
            isinstance(answer.get("data"), dict) and
            isinstance(answer["data"].get("version"), str)):
        return path
    return None


def homebrew_bin():
    brew = shutil.which("brew")
    if not brew:
        return None
    try:
        result = subprocess.run([brew, "--prefix"], capture_output=True,
                                text=True, timeout=5, check=False)
    except (OSError, subprocess.TimeoutExpired):
        return None
    prefix = result.stdout.strip()
    if result.returncode != 0 or not prefix or "\n" in prefix:
        return None
    directory = Path(prefix) / "bin"
    if not directory.is_absolute() or not directory.is_dir() or not os.access(directory, os.W_OK):
        return None
    if not any(Path(entry).expanduser().absolute() == directory for entry in
               os.environ.get("PATH", "").split(os.pathsep) if entry):
        return None
    occupant = directory / "rhost"
    if occupant.exists() or occupant.is_symlink():
        return None
    return directory


def install_directory(os_name):
    explicit = os.environ.get("RHOST_INSTALL_DIR")
    if explicit:
        return Path(explicit).expanduser()
    if os_name == "darwin":
        active = active_standalone_rhost()
        if active:
            return active.parent
        available = homebrew_bin()
        if available:
            return available
    return Path.home() / ".local/bin"


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
    install_dir = install_directory(os_name)
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
    installed = install_dir / "rhost"
    print(f"installed {installed}")
    selected = shutil.which("rhost")
    try:
        visible = selected is not None and os.path.samefile(selected, installed)
    except OSError:
        visible = False
    if not visible:
        if selected:
            print(f"note: 'rhost' still resolves to {selected}, not {installed}")
        else:
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
