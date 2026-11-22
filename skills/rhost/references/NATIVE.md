# Native interface and helper preparation

Read the guide for the operation in front of you:

- [RUNS.md](RUNS.md): recoverable execution, keys, deadlines and output cursors.
- [FILES.md](FILES.md): native file observations, CAS and named recovery.
- [MCP.md](MCP.md): stdio setup and structured requests.
- [SESSIONS.md](SESSIONS.md): persistent interactive terminals.

The native C99 helper owns file and Run primitives. It is separate from the
bash/tmux session implementation. `helper prepare` creates or validates a
namespace and returns its generation/time/identity; `helper status` is read-only
and refuses an absent namespace. Doctor reports command presence separately.

## Preparation policies


An explicit `RHOST_HELPER_BIN` selects exact local bytes. Alternatively,
`RHOST_HELPER_CC`, `RHOST_HELPER_SYSROOT`, `RHOST_HELPER_BUILTINS` and
`RHOST_HELPER_LD` select a fixed local LLVM/musl build for the observed Linux
architecture. Invalid/failed explicit configuration refuses without fallback.
Automatic preparation tries existing remote C99 `cc` and then a SHA-pinned
static Linux release binary. No packages or toolchains are automatically installed.

`RHOST_HELPER_PACK_DIR` takes precedence over a supplied binary or compiler.
It selects an exact offline artifact by target ABI and the client's helper
source digest. Put a `manifest.json` in the pack root with this structure.

```json
{"helpers":[{"target":"linux-aarch64","source_digest":"<source_digest>","binary_sha256":"<binary_sha256>"}]}
```

Use full lowercase SHA-256 values and one entry per target and source pair.
Under `<target>/<source_digest>/<binary_sha256>/`, store `rhost-helper` and
`rhost-helper.sha256`. The sidecar contains the binary digest, two spaces, and
`rhost-helper`. The manifest, sidecar and bytes must agree. A pack works with
both build and download disabled. Without a fixed path, preparation still
uploads the image to the private remote cache when needed.

`RHOST_TARGET_ABI` selects the userspace ABI explicitly, for example
`linux-aarch64`, `linux-x86_64`, or `linux-armv7-eabihf`. The observed OS
and the helper's compiled ABI must match. Without it, rhost infers the
target from `uname`.

`RHOST_REMOTE_HELPER_PATH` selects an existing executable on the remote
host and bypasses the helper cache. Use an absolute or remote-home-relative
path with no symlinks. The file must be regular, executable, owned by root or
the SSH account, and unwritable by group or others. Set `RHOST_HELPER_SHA256`
to pin its bytes. A configured pack also supplies a pin, and both pins must
agree when supplied. A pin requires a remote `sha256sum`, `sha256`, or
`shasum` command. Use a separately writable `RHOST_REMOTE_STATE` for native
state when the executable is on a read-only filesystem.

`RHOST_NO_UPLOAD` permits reuse of an existing helper. `RHOST_NO_SOURCE_BUILD`
disables remote source builds; `RHOST_NO_DOWNLOAD` disables release downloads.
`RHOST_HELPER_CACHE_DIR` chooses a private cache root. Neither the native helper
nor its operations require remote Python/tmux. Bootstrap still needs SSH's shell
and ordinary `sh`, `uname`, `id`, `stat`, `cat`, `chmod`, `mktemp` and `rm` tools.
