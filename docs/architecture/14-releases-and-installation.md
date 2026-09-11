[← Architecture map](../ARCHITECTURE.md)

# Part XIV — releases and installation

## 38. Release artifacts

Automate releases with GoReleaser or a small GitHub Actions matrix.

The matrix that ships does the following, in this order:

1. `make check` on the tagged commit — a tag is a claim, not evidence, and a red
   tree must not become a published binary;
2. one static binary per platform, with version, commit and build date linked in;
3. a `.sha256` file beside each binary;
4. one GitHub Release carrying the binaries and their checksum files.

CI runs the same gate plus a build on **ubuntu-latest and macos-latest**: the
remote side is Linux, the first local side is macOS, and OpenSSH, unix sockets,
rsync (GNU vs openrsync) and PATH differ between them.

A tag containing a hyphen (`v0.1.0-alpha.1`) is published as a GitHub
*prerelease*, and that is a statement about maturity rather than a hidden flag:
`scripts/install.sh` asks `/releases/latest` first and falls back to the newest
release of any kind while every release is a prerelease, so the one-line install
works before the first stable release exists.

Initial artifacts:

```text
darwin/arm64
darwin/amd64
linux/amd64
linux/arm64
```

Priority:

```text
1. darwin/arm64
2. linux/amd64
3. linux/arm64
4. darwin/amd64
```

The project should be runnable after copying one binary into `$PATH`.

The installer downloads the appropriate release asset together with its `.sha256`
file, verifies the digest with `sha256sum` or `shasum`, and only then replaces the
installed binary — atomically, within the install directory. A failed verification
leaves an existing install untouched. `RHOST_INSTALL_DIR` chooses the directory
(default `~/.local/bin`) and `RHOST_VERSION` pins a version.

Do not require Python, Node, or a virtual environment.

---

## 39. Version output

Provide:

```bash
rhost version
```

Include:

```text
version
git commit
build date
Go version
schema version
```

This is important when an Agent reports a remote-control behavior that may depend on CLI version.

---
