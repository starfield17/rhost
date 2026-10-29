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

`RHOST_NO_UPLOAD` allows compatible cache reuse only; `RHOST_NO_SOURCE_BUILD`
disables remote source builds; `RHOST_NO_DOWNLOAD` disables release downloads.
`RHOST_HELPER_CACHE_DIR` chooses a private cache root. Neither the native helper
nor its operations require remote Python/tmux. Bootstrap still needs SSH's shell
and ordinary `sh`, `uname`, `id`, `stat`, `cat`, `chmod`, `mktemp` and `rm` tools.
