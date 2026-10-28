# Proposed helper protocol v1

Status: acceptance design, not implemented. See
[the architecture proposal](agent-interface-v5.md). Public requests use schema
v3; these frames are a private Rust/C interface, not an alternate public API.

## Encoding

Each invocation accepts exactly one request frame followed by EOF and returns
one response frame. Header: ASCII `RH01`, big-endian unsigned 32-bit kind,
big-endian unsigned 64-bit body length. Header size is 16 bytes. Maximum body
length is 8 MiB. Incomplete, oversized or trailing input is rejected before any
mutation. A frame kind uniquely determines its body; fields cannot be repeated.

Primitive types:

| Type | Layout |
| --- | --- |
| u8 | one unsigned byte |
| u32/u64 | big-endian unsigned integer, no alignment padding |
| bytes | u32 length then that many bytes |
| text | bytes, strict UTF-8; embedded NUL refused |
| boolean | u8, exactly zero or one |
| texts | u32 count then that many text fields |

Bounds are checked before allocation or arithmetic. A count cannot exceed the
remaining body length divided by the minimum element size. Texts contain at most
4096 entries. Keys and generations use the public schema's ASCII grammar.
Integer conversion overflow is a protocol error. No C structure is serialized
by memory layout. No input field is evaluated as shell syntax.

JSON Schema cannot enforce cross-field comparisons: the Rust/domain boundary
must check that key expiry equals expires_unix and that stream cursor ordering
and decoded content length agree. These require executable parser/domain tests
before production command wiring, not schema-only fixtures.

## Request bodies

Fields are listed in wire order. Kind zero is reserved and invalid.

| Kind | Name | Body |
| --- | --- | --- |
| 1 | probe | empty |
| 2 | start | generation:text, key:text, expires:u64, cwd:text, program:u8, program fields, inherit:boolean, env count:u32, repeated name:text/value:text pairs, stdin:bytes, wait_ms:u32, deadline_ms:u64 |
| 3 | poll | generation:text, run_id:text, wait_ms:u32, stdout_since:u64, stderr_since:u64, max_bytes:u32 |
| 4 | list | generation:text |
| 5 | signal | generation:text, run_id:text, signal:u8 |
| 6 | prune | generation:text |
| 7 | prepare namespace | empty; authorizes creating private namespace state |
| 8 | namespace metadata | empty; refuses an absent namespace without creating it |

Start program tag 1 means argv:texts; argv[0] is nonempty. Tag 2 means
interpreter:text, script:text, login:boolean. Other tags are invalid. Environment
names are unique valid identifiers, ordered by ASCII bytes; duplicates or
out-of-order names are refused. Empty stdin means closed input after zero bytes.
Zero deadline means no deadline. Missing public wait defaults to zero; poll page
defaults to 4096 bytes. Signal tags 1/2/3 mean INT/TERM/KILL, independent of
platform signal numbers. Receipt of a signal response proves delivery only.

## Response bodies

Response kind is request kind plus 2147483648. First body field is result:u8.
Zero means success followed by the operation payload below. One means refusal
followed by code:text, message:text, retryable:boolean. Other tags are invalid.
The outer Rust layer retains a previously validated observation when transport
fails; the helper never invents a stale observation from current transport data.

| Request | Success payload |
| --- | --- |
| probe | source_digest:text, target_os:text, target_arch:text, capabilities:u32 |
| start/poll | observation |
| list | count:u32 followed by that many run records |
| signal | run_id:text, delivered:boolean (must be true) |
| prune | removed:texts |
| prepare namespace / namespace metadata | generation:text, remote_time_unix:u64 |

A run record is generation:text, run_id:text, state:u8, and an optional
completion. States 1/2/3/4/5 mean accepted/starting/running/exited/lost. Only
exited includes completion: kind:u8 (1 exit code, 2 terminating signal), value:u32.
Exit code is at most 255; signal is nonzero. Other states have no completion
bytes. A list contains at most 4096 records; excess records cause a bounded
refusal rather than an apparently complete partial list.

An observation is one run record then stdout:stream and stderr:stream. A stream
is from:u64, next:u64, observed_bytes:u64, content:bytes, open:boolean,
incomplete:boolean. Require next >= from, next - from == content length and
next <= observed_bytes. Content is raw bytes, base64-encoded only at the public
JSON boundary. A requested offset in a discarded range is refused explicitly,
never silently advanced; incomplete marks known discarded or unobserved output.

Probe capabilities are a bitset: bit 0 recoverable Runs, bit 1 validated group
signaling, bit 2 file CAS. Unknown bits must be ignored by the consumer and never
inferred from source compilation alone. A helper must not advertise a capability
whose readiness checks failed. The protocol version is the header magic; source
digest is 64 lowercase hex characters. Preparation matches both before reuse.
Probe never initializes a namespace. Namespace preparation is a separate
authorized operation; an executable handshake cannot manufacture a generation.

The executable accepts a private `--install-cache <absolute-directory>` bootstrap
entry point. It copies its own regular executable into an account-owned private
directory using a lock, no-follow opens and atomic publication. This entry point
does not initialize namespaces or launch work. The SSH bootstrap compiles only
embedded source in a private temporary directory and invokes this installer.

Run status records use RHST0001 plus state:u8, completion-kind:u8, value:u32,
stdout-open:u8, stderr-open:u8, incomplete:u8, stdout-observed:u64,
stderr-observed:u64, stdout-retained:u64, stderr-retained:u64. The fixed size is
49 bytes. A lifetime owner.lock is held by the monitor, never its exec child.
The request fingerprint hashes the canonical start body excluding wait_ms.

## Parser acceptance cases

Before the parser is shipped, Rust black-box fixtures must exercise every
request/success/refusal kind plus: partial header, wrong magic, excessive length,
truncated field, overlong count, integer overflow, unknown enum, invalid boolean,
invalid UTF-8, embedded NUL, duplicate environment name, trailing frame, trailing
body bytes, impossible completion and inconsistent stream cursors. Malformed
requests must demonstrate no filesystem or child-process side effects.

Control handling and non-reaping child-exit observation share one serialized
monitor event loop. Once control is disabled, queued controls are refused before
the leader is reaped. State-directory restoration is outside the deduplication
guarantee: restoring its generation file cannot detect that restoration itself.
