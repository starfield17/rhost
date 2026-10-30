# Recoverable execution

Use a Run for noninteractive work that must remain discoverable after the local
CLI or MCP server exits. Foreground exec keeps its own timeout/cleanup contract;
interactive terminal state belongs to [sessions](SESSIONS.md).

## Prepare and retain identity


```bash
rhost helper prepare gpu --json
rhost helper status gpu --json
```

Preparation returns `data.generation`, `data.remote_time_unix`, optional
`data.context.boot_id`, and `data.helper` with source/target/protocol/limits and
actual executable SHA. Status is read-only and refuses an absent namespace.
Doctor's command-presence map is separate from native readiness.

Generate a random key before submission:
`<expiry-unix-seconds>-<64-lowercase-hex-random-characters>`. Derive expiry from
observed remote time, at most seven days ahead, and retain generation/key/payload
before submitting. Namespace replacement or restoring old state invalidates
deduplication history. Expired keys cannot launch again after pruning.

```bash
rhost run start gpu --generation '<generation>' --key '<expiring-key>' \
  --cwd '~/work/project' --argv-json '["/bin/sh","-c","printf finished"]' \
  --wait 10s --deadline 30m --json
rhost run poll gpu '<run-id>' --generation '<generation>' \
  --wait 30s --stdout-since 0 --stderr-since 0 --max-bytes 4096 --json
rhost run signal gpu '<run-id>' --generation '<generation>' --sig TERM --json
rhost run list gpu --generation '<generation>' --json
rhost run prune gpu --generation '<generation>' --json
```

`--wait` ends observation; it never stops a Run. Its default is zero, and an
accepted/starting result is valid before readiness. `--deadline` is enforced by
the remote monitor. Signals are explicit `INT`, `TERM` or `KILL`. Canceling the
local CLI or MCP observer leaves remote work alone.

Use absolute program paths and explicit `--env KEY=VALUE` where reproducibility
matters. Environment inheritance defaults to false; `--inherit-env` explicitly
inherits the SSH invocation environment. Shell programs use `--command`, an
absolute `--shell` (default `/bin/sh`), and optional `--login`. No profile is
implicitly loaded for argv execution. This is an explicit environment contract,
not a hermetic environment guarantee.

## Observation and retry

Inspect `data.run.status`: `accepted`, `starting`, `running`, `exited` or `lost`.
Only `exited` has `data.run.completion.exit_code` or `.signal`. `ok:true` means
the observation succeeded, including a nonzero child exit; inspect completion.

`data.stdout` and `data.stderr` each contain `from`, `next`, `observed_bytes`,
`content_base64`, displayed `content`, `open` and `incomplete`. Cursors count
source bytes. Repeat a retained offset to get the same bytes; advance with
`next`. Do not derive offsets from text length. Retention is 64 MiB per stream;
quota/write failure keeps draining/counting but reports gaps. Leader exit and
stream EOF are separate facts. Descendants are not a contained process tree.

An unknown submission is safely *deduplicated* only by the same still-valid
generation/key/payload. A changed payload returns `IDEMPOTENCY_CONFLICT`.
After expiry, poll the existing Run rather than trying to resubmit it. Never
turn `lost`, SSH failure, missing output or a vanished process into success or
permission to start a second side effect. Signal refuses without a live owner.
Prune explicitly removes only eligible expired completed records and ends their
retention window. There is no automatic restart, scheduler or background GC.

## MCP submission

Use `rhost_helper_prepare` with `host` and `params: {}`. Retain generation and
remote time, then generate and retain a key and immutable payload before calling
`rhost_run_start`. The tool uses the same closed params as a JSON request. This is a template;
substitute the retained values and integer expiry before calling:

```text
{"host":"gpu","params":{"generation":"<generation>","key":"<expiring-key>","expires_unix":<expiry-unix-seconds>,"cwd":"~/work/project","program":{"argv":["/usr/bin/make","test"]},"environment":{"inherit":false,"values":{}},"wait_ms":10000}}
```

Replace the illustrative expiry placeholder with an integer matching the key.
Use `rhost_run_poll` with generation/run_id and stream cursors to reconnect.
A nonzero child exit can accompany `ok:true`; inspect `data.run.completion`.
Bounded key deduplication does not make external side effects exactly once.
For helper build/cache policies, load [NATIVE.md](NATIVE.md#preparation-policies)
only when needed.

## Rediscover retained work

`run.list` is a bounded page object with `items` and `next_cursor`, not an array.
Pass generation, optional limit (1–256) and the returned cursor to continue.
Inspect per-item evidence errors and use `run.poll` for full Run/output evidence.
For namespace usage and explicit prune preview, read
[FILES.md](FILES.md#state-discovery-and-retention).
