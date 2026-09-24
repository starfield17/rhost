export RHOST_BIN
export RHOST_BIN_ROOT := $(CURDIR)
export RHOST_REPO_ROOT := $(CURDIR)

# Build provenance. `version` names what the binary was built from (CONTRACT.md
# RELEASE-001 "version result"); override COMMIT=/DATE= to pin it, which is what
# a reproducible build does.
COMMIT ?= $(shell git rev-parse --short HEAD 2>/dev/null || echo none)
DATE   ?= $(shell date -u +%Y-%m-%dT%H:%M:%SZ)
.PHONY: build build-release check rust-check test test-smoke stress-acceptance fmt portability structure live-suite release-contract contract-evidence agent-package install-test
build:
	RHOST_BUILD_COMMIT="$(COMMIT)" RHOST_BUILD_DATE="$(DATE)" cargo build --locked --bin rhost
# The release candidate: the same locked, optimized, provenance-stamped build
# the release workflow runs, and the binary the live gates exercise after
# `make build`. `make build` stays the debug build for everyday work.
build-release:
	RHOST_BUILD_COMMIT="$(COMMIT)" RHOST_BUILD_DATE="$(DATE)" cargo build --locked --release --bin rhost
fmt:
	cargo fmt
test:
	cargo test --locked
check: rust-check portability structure live-suite release-contract contract-evidence agent-package install-test
rust-check:
	cargo fmt --check
	cargo clippy --locked --all-targets -- -D warnings
	cargo clippy --locked --features live-tests --test live -- -D warnings
	cargo test --locked
	cargo test --locked --features live-tests --test live --no-run
	@mkdir -p target/domain-check
	rustc --edition=2024 --crate-type lib --crate-name rhost_domain src/domain/mod.rs -D warnings --out-dir target/domain-check
portability:
	./scripts/check-portability.sh

structure:
	./scripts/check-structure.sh

live-suite:
	./scripts/check-live-suite.sh

release-contract:
	./scripts/check-release-contract.sh
contract-evidence:
	./scripts/check-contract-evidence.sh
agent-package:
	./scripts/check-agent-package.sh
install-test:
	./scripts/test-install.sh
test-smoke:
	@command -v python3 >/dev/null 2>&1 || (echo "test-smoke requires python3 for the embedded remote-helper paths" >&2; exit 1)
	cargo test --locked --test acceptance

STRESS_RUNS ?= 50
stress-acceptance:
	@case "$(STRESS_RUNS)" in ''|*[!0-9]*|0) echo "STRESS_RUNS must be a positive integer" >&2; exit 2 ;; esac
	@i=0; while [ "$$i" -lt "$(STRESS_RUNS)" ]; do \
		i=$$((i + 1)); \
		echo "stress-acceptance: acceptance run $$i/$(STRESS_RUNS)"; \
		cargo test --locked --test acceptance || exit; \
	done

.PHONY: test-live-smoke
test-live-smoke:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=<user>@<host>" >&2; exit 1)
	@test -n "$(RHOST_BIN)" || (echo "set RHOST_BIN to an explicit binary" >&2; exit 1)
	cargo test --locked --features live-tests --test live 'smoke::' -- --nocapture --test-threads=1

.PHONY: test-live test-live-exec test-live-transport
test-live: test-live-exec test-live-transport
test-live-exec:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=<user>@<host>" >&2; exit 1)
	@test -n "$(RHOST_BIN)" || (echo "set RHOST_BIN to an explicit binary" >&2; exit 1)
	cargo test --locked --features live-tests --test live 'exec::' -- --nocapture --test-threads=1

test-live-transport:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=<user>@<host>" >&2; exit 1)
	@test -n "$(RHOST_BIN)" || (echo "set RHOST_BIN to an explicit binary" >&2; exit 1)
	cargo test --locked --features live-tests --test live 'transport::' -- --nocapture --test-threads=1

.PHONY: test-live-session
test-live-session:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=<user>@<host>" >&2; exit 1)
	@test -n "$(RHOST_BIN)" || (echo "set RHOST_BIN to an explicit binary" >&2; exit 1)
	cargo test --locked --features live-tests --test live 'session::' -- --nocapture --test-threads=1

.PHONY: test-live-fs
test-live-fs:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=<user>@<host>" >&2; exit 1)
	@test -n "$(RHOST_BIN)" || (echo "set RHOST_BIN to an explicit binary" >&2; exit 1)
	cargo test --locked --features live-tests --test live 'files::' -- --nocapture --test-threads=1

.PHONY: test-live-tools test-live-tunnel test-live-audit
test-live-tools: test-live-fs test-live-tunnel test-live-audit
test-live-tunnel:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=<user>@<host>" >&2; exit 1)
	@test -n "$(RHOST_BIN)" || (echo "set RHOST_BIN to an explicit binary" >&2; exit 1)
	cargo test --locked --features live-tests --test live 'tunnel::' -- --nocapture --test-threads=1

test-live-audit:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=<user>@<host>" >&2; exit 1)
	@test -n "$(RHOST_BIN)" || (echo "set RHOST_BIN to an explicit binary" >&2; exit 1)
	cargo test --locked --features live-tests --test live 'audit::' -- --nocapture --test-threads=1

.PHONY: test-live-all
test-live-all:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=<user>@<host>" >&2; exit 1)
	@test -n "$(RHOST_BIN)" || (echo "set RHOST_BIN to an explicit binary" >&2; exit 1)
	cargo test --locked --features live-tests --test live -- --nocapture --test-threads=1
