#!/usr/bin/env python3
"""Exercise both recognized installer formats without contacting GitHub."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
INSTALLER = ROOT / "scripts/install.sh"


def check(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def make_old_fakes(bin_dir: Path) -> None:
    tools = {
        "uname": '''#!/bin/sh
case "$1" in -s) printf '%s\n' "$RHOST_TEST_UNAME_S";; -m) printf '%s\n' "$RHOST_TEST_UNAME_M";; *) exit 2;; esac
''',
        "curl": '''#!/bin/sh
url=""; out=""
while [ "$#" -gt 0 ]; do
  case "$1" in -o) out=$2; shift 2;; -*) shift;; *) url=$1; shift;; esac
done
printf '%s\n' "$url" >> "$RHOST_TEST_CURL_LOG"
case "$url" in
  */releases/latest) [ "${RHOST_TEST_LATEST_FAIL:-0}" = 1 ] && exit 22; printf '%s\n' '{"tag_name":"v9.8.7"}';;
  *'/releases?per_page=1') printf '%s\n' '[{"tag_name":"v2.0.0-alpha.1"}]';;
  *.sha256) checksum_asset=${url##*/}; checksum_asset=${checksum_asset%.sha256}; printf '%s  %s\n' 'placeholder' "$checksum_asset" > "$out";;
  *) printf '%s\n' 'verified binary' > "$out";;
esac
''',
        "sha256sum": '''#!/bin/sh
[ "${RHOST_TEST_BAD_CHECKSUM:-0}" != 1 ] || exit 1
[ "$1" = -c ] || exit 2
file=$2
asset=$(sed -n 's/^[^ ]*  //p' "$file")
[ -n "$asset" ] && [ -f "$asset" ]
''',
    }
    for name, body in tools.items():
        path = bin_dir / name
        path.write_text(body)
        path.chmod(0o755)


def make_python_hook(directory: Path) -> None:
    (directory / "sitecustomize.py").write_text('''import hashlib
import io
import os
import platform
import urllib.error
import urllib.request

platform.system = lambda: os.environ.get("RHOST_TEST_UNAME_S", "Linux")
platform.machine = lambda: os.environ.get("RHOST_TEST_UNAME_M", "x86_64")

class Response(io.BytesIO):
    def __init__(self, data, url):
        super().__init__(data)
        self.url = url
    def geturl(self):
        return self.url
    def __enter__(self):
        return self
    def __exit__(self, *args):
        self.close()

def urlopen(request, *args, **kwargs):
    url = request.full_url if hasattr(request, "full_url") else request
    with open(os.environ["RHOST_TEST_CURL_LOG"], "a") as log:
        log.write(url + "\\n")
    if url.endswith("/releases/latest"):
        if os.environ.get("RHOST_TEST_LATEST_FAIL") == "1":
            raise urllib.error.HTTPError(url, 404, "no stable release", {}, None)
        return Response(b"", "https://github.com/starfield17/rhost/releases/tag/v9.8.7")
    if "/releases?per_page=1" in url:
        if os.environ.get("RHOST_TEST_API_RATE_LIMIT") == "1":
            raise urllib.error.HTTPError(url, 403, "rate limit", {"x-ratelimit-remaining": "0"}, None)
        return Response(b'[{"tag_name":"v2.0.0-alpha.1"}]', url)
    if url.endswith(".sha256"):
        asset = url.rsplit("/", 1)[-1][:-7]
        digest = "0" * 64 if os.environ.get("RHOST_TEST_BAD_CHECKSUM") == "1" else hashlib.sha256(b"verified binary\\n").hexdigest()
        return Response(f"{digest}  {asset}\\n".encode(), url)
    return Response(b"verified binary\\n", url)

urllib.request.urlopen = urlopen
''')


def run(root: Path, target: Path | None, mode: str, **values: str) -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    env.update({"HOME": str(root / "home"), "RHOST_TEST_CURL_LOG": str(root / "requests.log")})
    env.pop("RHOST_INSTALL_DIR", None)
    if target is not None:
        env["RHOST_INSTALL_DIR"] = str(target)
    env.update(values)
    if mode == "old":
        env["PATH"] = f"{root / 'bin'}:/usr/bin:/bin"
    else:
        env["PYTHONPATH"] = str(root / "hooks")
        if "PATH" not in values:
            env["PATH"] = f"{Path(sys.executable).parent}:{root / 'bin'}:/usr/bin:/bin"
    return subprocess.run(["bash", str(INSTALLER)], env=env, capture_output=True, text=True)


def main() -> int:
    source = INSTALLER.read_text()
    mode = "python" if "PY_INSTALLER" in source else "old"
    with tempfile.TemporaryDirectory(prefix="rhost-install-test-") as tmp:
        root = Path(tmp)
        for name in ("home", "bin", "hooks"):
            (root / name).mkdir()
        make_old_fakes(root / "bin")
        make_python_hook(root / "hooks")
        platforms = [
            ("Darwin", "x86_64", "darwin_amd64"), ("Darwin", "arm64", "darwin_arm64"),
            ("Linux", "x86_64", "linux_amd64"), ("Linux", "aarch64", "linux_arm64"),
        ]
        for os_name, machine, suffix in platforms:
            (root / "requests.log").write_text("")
            target = root / suffix
            result = run(root, target, mode, RHOST_VERSION="v1.2.3",
                         RHOST_TEST_UNAME_S=os_name, RHOST_TEST_UNAME_M=machine)
            check(result.returncode == 0, f"{suffix}: {result.stderr}")
            check(f"/download/v1.2.3/rhost_1.2.3_{suffix}" in (root / "requests.log").read_text(),
                  f"{suffix}: wrong asset URL")
            check((target / "rhost").read_text() == "verified binary\n", f"{suffix}: wrong payload")
            check("not on your PATH" in result.stdout and f'export PATH="{target}:' in result.stdout,
                  f"{suffix}: missing PATH advice")
        (root / "requests.log").write_text("")
        stable = run(root, root / "stable", mode, RHOST_TEST_UNAME_S="Linux",
                     RHOST_TEST_UNAME_M="x86_64")
        check(stable.returncode == 0, stable.stderr)
        requests = (root / "requests.log").read_text()
        check("/releases/latest" in requests and "/download/v9.8.7/" in requests,
              "stable release lookup lost")
        if mode == "python":
            check("api.github.com" not in requests, "stable release used the REST API")
        (root / "requests.log").write_text("")
        pre = run(root, root / "prerelease", mode, RHOST_TEST_LATEST_FAIL="1",
                  RHOST_TEST_UNAME_S="Linux", RHOST_TEST_UNAME_M="x86_64")
        check(pre.returncode == 0, pre.stderr)
        check("/download/v2.0.0-alpha.1/rhost_2.0.0-alpha.1_linux_amd64" in
              (root / "requests.log").read_text(), "prerelease fallback lost")
        rollback = root / "rollback"
        rollback.mkdir()
        (rollback / "rhost").write_text("existing binary\n")
        bad = run(root, rollback, mode, RHOST_VERSION="1.2.3", RHOST_TEST_BAD_CHECKSUM="1",
                  RHOST_TEST_UNAME_S="Linux", RHOST_TEST_UNAME_M="x86_64")
        check(bad.returncode != 0 and (rollback / "rhost").read_text() == "existing binary\n",
              "checksum failure replaced existing binary")
        unsupported = run(root, root / "unsupported", mode, RHOST_VERSION="1.2.3",
                          RHOST_TEST_UNAME_S="Unsupported", RHOST_TEST_UNAME_M="x86_64")
        check(unsupported.returncode != 0, "unsupported OS installed")
        if mode == "python":
            active = root / "bin/rhost"
            active.write_text('''#!/bin/sh
if [ "$1" = version ] && [ "$2" = --json ]; then
  printf '%s\\n' '{"schema_version":2,"operation":"version","ok":true,"data":{"version":"1.0.0"},"error":null}'
else
  exit 2
fi
''')
            active.chmod(0o755)
            updated = run(root, None, mode, RHOST_VERSION="1.2.3",
                          RHOST_TEST_UNAME_S="Darwin", RHOST_TEST_UNAME_M="arm64")
            check(updated.returncode == 0 and active.read_text() == "verified binary\n",
                  f"default install did not update the active standalone binary: {updated.stderr}")
            check("not on your PATH" not in updated.stdout,
                  "installer warned after updating the active binary")
            active.unlink()
            managed = root / "managed"
            managed.mkdir()
            (managed / "rhost").write_text("managed binary\n")
            (managed / "rhost").chmod(0o755)
            active.symlink_to(managed / "rhost")
            linked = run(root, None, mode, RHOST_VERSION="1.2.3",
                         RHOST_TEST_UNAME_S="Darwin", RHOST_TEST_UNAME_M="arm64")
            check(linked.returncode == 0 and active.is_symlink() and
                  (managed / "rhost").read_text() == "managed binary\n",
                  "installer replaced a managed symlink")
            check((root / "home/.local/bin/rhost").read_text() == "verified binary\n" and
                  str(active) in linked.stdout, "shadowing executable was not reported")
            active.unlink()
            homebrew = root / "homebrew/bin"
            homebrew.mkdir(parents=True)
            brew = root / "bin/brew"
            brew.write_text(f'#!/bin/sh\nprintf "%s\\n" "{root / "homebrew"}"\n')
            brew.chmod(0o755)
            fresh = run(root, None, mode, RHOST_VERSION="1.2.3",
                        RHOST_TEST_UNAME_S="Darwin", RHOST_TEST_UNAME_M="arm64",
                        PATH=f"{Path(sys.executable).parent}:{homebrew}:{root / 'bin'}:/usr/bin:/bin")
            check(fresh.returncode == 0 and (homebrew / "rhost").read_text() == "verified binary\n",
                  f"fresh macOS install did not use the available Homebrew bin: {fresh.stderr}")
            active.write_text('''#!/bin/sh
printf '%s\\n' '{"schema_version":2,"operation":"version","ok":true,"data":{"version":"1.0.0"},"error":null}'
''')
            active.chmod(0o755)
            explicit = run(root, root / "explicit", mode, RHOST_VERSION="1.2.3")
            check(explicit.returncode == 0 and (root / "explicit/rhost").exists() and
                  active.read_text().startswith("#!/bin/sh"),
                  "explicit install directory did not take precedence")
            linux = run(root, None, mode, RHOST_VERSION="1.2.3",
                        RHOST_TEST_UNAME_S="Linux", RHOST_TEST_UNAME_M="x86_64")
            check(linux.returncode == 0 and active.read_text().startswith("#!/bin/sh") and
                  (root / "home/.local/bin/rhost").exists(),
                  "Linux default unexpectedly replaced the active binary")
            (root / "requests.log").write_text("")
            pinned = run(root, root / "pinned", mode, RHOST_VERSION="1.2.3")
            check(pinned.returncode == 0, pinned.stderr)
            check("/releases/latest" not in (root / "requests.log").read_text() and
                  "api.github.com" not in (root / "requests.log").read_text(),
                  "explicit version queried release discovery")
            on_path = run(root, root / "bin", mode, RHOST_VERSION="1.2.3",
                          PATH=f"{root / 'bin'}:{os.environ['PATH']}")
            check(on_path.returncode == 0 and "not on your PATH" not in on_path.stdout,
                  "installer warned about an installation directory already on PATH")
            limited = run(root, root / "limited", mode, RHOST_TEST_LATEST_FAIL="1",
                          RHOST_TEST_API_RATE_LIMIT="1")
            check(limited.returncode != 0 and "403" in limited.stderr and
                  "RHOST_VERSION" in limited.stderr, "API rate-limit recovery missing")
            check(not (root / "limited/rhost").exists(), "rate-limit failure installed")
            piped = subprocess.run(["bash"], input=source, env={**os.environ,
                    "PYTHONPATH": str(root / "hooks"), "RHOST_INSTALL_DIR": str(root / "piped"),
                    "RHOST_VERSION": "1.2.3", "RHOST_TEST_CURL_LOG": str(root / "requests.log")},
                    capture_output=True, text=True)
            check(piped.returncode == 0 and (root / "piped/rhost").exists(),
                  f"single-file pipe entry failed: {piped.stderr}")
    print("test-install: OK")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except AssertionError as error:
        print(f"test-install: {error}", file=sys.stderr)
        sys.exit(1)
