# AGENTS.md — rules for anyone (human or agent) working on this repository

Hard rules, not style preferences.

## 1. Nothing local-only goes into tracked content

Everything tracked — `README.md`, `skills/rhost/SKILL.md`, `docs/**`, code comments, test
source, commit messages, example output — must be true for *any* SSH-reachable
host. So information about one specific machine, account, network, or test box
stays out:

- hardware model names, on either the client or the remote side
- private/RFC1918 addresses and `.local` mDNS names
- real SSH logins, real home paths, personal `~/.ssh/config` aliases
- secrets: keys, tokens, passwords, host keys, fingerprints — ever

Write generic classes instead: `local machine`, `remote Linux host`,
`user@example-host`, `/home/<user>/<project>`, placeholder aliases like `gpu`.
Generic platform names are fine (`Linux`, `WSL2`, `darwin/arm64`, `tmux`).
Verification is a claim about the software — `verified against a real remote
Linux host over SSH`, never `verified from <my box> to <my board>`. No real test
logs, config excerpts, or transcripts; illustrative placeholder scenarios are fine.

A real target belongs in your shell history, in untracked scratch (`local/`, your
own `~/.ssh/config`), or in the environment at run time — the only supported way:

```bash
RHOST_TEST_HOST=<user>@<host> make test-live-all   # never a hardcoded default
```

Anything needing a target reads it from the environment and fails with a clear
"set `RHOST_TEST_HOST`" message when unset.

`./scripts/check-portability.sh` (in CI, and in `make check`) enforces this and
holds the authoritative pattern list: when the test hardware changes, add its
model family there and prove the check by making it fail once before restoring.

## 2. Commit only your own changes; never push unasked

`git status` first; stage by explicit path, never `git add -A` or `.`. Changes you
didn't make: report and ask, don't touch. No push, tag, force-push, or amend of a
pushed commit without an explicit request.

## 3. Do not touch `reference/`

Third-party reference source, git-ignored. Read for ideas; never edit, commit, or
copy its product shape (see [Repository boundaries](docs/architecture/engineering.md#repository-boundaries)).

## 4. The persistence invariant

Anything promised to survive a CLI invocation must be owned outside the CLI
process: connections by OpenSSH ControlMaster and sessions by remote tmux and
remote state. The CLI owns nothing durable; no local Go map may
be the only copy of persistent state, and no hidden `rhost` server.

## 5. OpenSSH owns auth and host keys

Never disable host-key checking, persist keys or passwords, reimplement SSH config
resolution, or auto-install remote packages.

## 6. Every agent-visible behaviour needs a JSON path

Anything rendered for humans must also appear under `--json` in the versioned
envelope, with no colour or progress output on stdout. Agents branch on
`error.code`, never on English text.

## 7. Honesty in status claims

Document only what is implemented *and* verified; list the rest as planned. Code
existing isn't a milestone — persistence isn't implemented until it survives a
real process exit and reconnect on a real remote host.

## 8. No abstraction before a second implementation needs it

No `utils/` dumping ground, no interfaces for tidiness, no training/dataset/model
concepts in a generic adapter, no daemon until real usage justifies one
(see [Product rules](docs/ARCHITECTURE.md#product-rules) and
[Repository boundaries](docs/architecture/engineering.md#repository-boundaries)).

## 9. Verify before reporting

```bash
make check                                                # gofmt + vet + tests + portability + release contract
RHOST_TEST_HOST=<user>@<host> make test-live              # exec + doctor
RHOST_TEST_HOST=<user>@<host> make test-live-session      # sessions
RHOST_TEST_HOST=<user>@<host> make test-live-all          # every live suite
```

Name in your report exactly which suite you ran, and don't widen a test script's
or CI job's run-pattern to cover tests you weren't asked to run.
