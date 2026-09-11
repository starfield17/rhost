[← Architecture map](../ARCHITECTURE.md)

# Part XIV — releases and installation

## 38. Release artifacts

Releases are built by a small GitHub Actions matrix, not by a release tool, and
`Makefile`'s `dist` target is the only place an artifact is produced: a laptop and
CI run the same recipe, so the names and the linked-in metadata cannot disagree.

The pipeline runs in this order:

1. `make check` on the tagged commit, on both local platforms
   (`ubuntu-24.04` and `macos-15`) — a tag is a claim, not evidence, and a red
   tree must not become a published binary. The same gate, on the same two
   labels, runs for every push to `main` and every pull request: the remote side
   is Linux, the first local side is macOS, and OpenSSH, unix sockets, rsync
   (GNU vs openrsync) and PATH differ between them;
2. one Go cross-compile on one runner produces every artifact. Go cross-compiles
   by construction, so the runners below never rebuild what they are about to
   check. Each binary carries version, commit and build date linked in, with a
   `.sha256` file beside it;
3. each artifact is downloaded by a runner of its own platform and architecture
   (`macos-15-intel`, `macos-15`, `ubuntu-24.04`, `ubuntu-24.04-arm`) and
   *executed* there: it must report the release's version, the tagged commit and
   a well-formed JSON envelope, and its checksum file must verify the bytes it
   arrived as. A cross-compiled binary that never ran is not evidence;
4. the artifacts receive a build-provenance attestation
   (`actions/attest-build-provenance`), naming the workflow, repository and
   commit behind the bytes. A download is checked with
   `gh attestation verify <asset> --repo starfield17/rhost`. This is provenance,
   not a project signature;
5. a tag push then publishes the GitHub Release, and only after the expected
   asset set has been re-derived from `RELEASE_TARGETS` and checked to be exactly
   the four binaries and their four checksum files. A missing, extra or renamed
   asset fails the release instead of shipping quietly, and re-running the
   workflow replaces the assets instead of duplicating them.

A dispatched run (`workflow_dispatch`) runs the same pipeline and stops before
publishing, so the build, the four native runs and the attestation can be proven
before a tag exists. Only a tag push can create a release.

Tags must be `vMAJOR.MINOR.PATCH[-prerelease]`; the build refuses any other shape,
because `install.sh` names the asset after the tag and a tag it cannot name is a
broken install.

A tag containing a hyphen (`v0.1.0-alpha.1`) is published as a GitHub
*prerelease*, and that is a statement about maturity rather than a hidden flag:
`scripts/install.sh` asks `/releases/latest` first and falls back to the newest
release of any kind while every release is a prerelease, so the one-line install
works before the first stable release exists.

The four shipping targets, and the set `scripts/check-release-contract.sh` keeps
the Makefile, the release workflow and the installer agreeing on:

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
