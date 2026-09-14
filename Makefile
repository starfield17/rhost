export RHOST_BIN
export RHOST_BIN_ROOT := $(CURDIR)
export RHOST_REPO_ROOT := $(CURDIR)
.PHONY: build build-rust check rust-check test test-smoke fmt portability structure live-suite release-contract agent-package install-test build-reference test-conformance
build:
	cargo build --locked --bin rhost
build-rust: build
fmt:
	cargo fmt
test:
	cargo test --locked
check: rust-check portability structure live-suite release-contract agent-package install-test
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
agent-package:
	./scripts/check-agent-package.sh
install-test:
	./scripts/test-install.sh
build-reference:
	@mkdir -p bin
	cd archive/go-v3.1.0 && go build -trimpath -ldflags "-X github.com/starfield17/rhost/internal/buildinfo.Version=3.1.0 -X github.com/starfield17/rhost/internal/buildinfo.Commit=d9daaa9" -o ../../bin/rhost-go ./cmd/rhost
test-conformance:
	@test -n "$(RHOST_BIN)" || (echo "set RHOST_BIN to an explicit binary" >&2; exit 1)
	cd archive/conformance-v1 && RHOST_TEST_CONFORMANCE=1 go test ./conformance -run 'TestConformance|TestSchema|TestHarness|TestLiveCorpusIntegrity|TestHistoricalEvidenceLinks' -count=1 -timeout 5m

test-smoke:
	@command -v python3 >/dev/null 2>&1 || (echo "test-smoke requires python3 for the embedded remote-helper paths" >&2; exit 1)
	cargo test --locked --test acceptance

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

.PHONY: test-live-rust
test-live-rust: test-live-all

.PHONY: test-legacy-live-smoke test-legacy-live test-legacy-live-session test-legacy-live-fs test-legacy-live-tools test-legacy-live-all
test-legacy-live-smoke:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=<user>@<host>" >&2; exit 1)
	@test -n "$(RHOST_BIN)" || (echo "set RHOST_BIN to an explicit binary" >&2; exit 1)
	cd archive/conformance-v1 && RHOST_TEST_LIVE=1 go test ./conformance -run '^TestSmokeLive$$' -v -count=1 -parallel 3 -timeout 5m

test-legacy-live:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=<user>@<host>" >&2; exit 1)
	@test -n "$(RHOST_BIN)" || (echo "set RHOST_BIN to an explicit binary" >&2; exit 1)
	cd archive/conformance-v1 && RHOST_TEST_LIVE=1 go test ./conformance -run 'TestLiveExec|TestLiveDoctor|TestLiveTransportReuse|TestLiveControlPath' -v -count=1 -parallel 3 -timeout 10m

test-legacy-live-session:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=<user>@<host>" >&2; exit 1)
	@test -n "$(RHOST_BIN)" || (echo "set RHOST_BIN to an explicit binary" >&2; exit 1)
	cd archive/conformance-v1 && RHOST_TEST_LIVE=1 go test ./conformance -run 'TestLiveSession' -v -count=1 -parallel 3 -timeout 10m

test-legacy-live-fs:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=<user>@<host>" >&2; exit 1)
	@test -n "$(RHOST_BIN)" || (echo "set RHOST_BIN to an explicit binary" >&2; exit 1)
	cd archive/conformance-v1 && RHOST_TEST_LIVE=1 go test ./conformance -run 'TestLiveFs' -v -count=1 -parallel 3 -timeout 20m

test-legacy-live-tools:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=<user>@<host>" >&2; exit 1)
	@test -n "$(RHOST_BIN)" || (echo "set RHOST_BIN to an explicit binary" >&2; exit 1)
	cd archive/conformance-v1 && RHOST_TEST_LIVE=1 go test ./conformance -run 'TestLiveTools|TestLiveTunnel' -v -count=1 -parallel 3 -timeout 20m

test-legacy-live-all:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=<user>@<host>" >&2; exit 1)
	@test -n "$(RHOST_BIN)" || (echo "set RHOST_BIN to an explicit binary" >&2; exit 1)
	cd archive/conformance-v1 && RHOST_TEST_LIVE=1 go test ./conformance -run '^TestLive' -v -count=1 -parallel 3 -timeout 20m
