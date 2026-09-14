export RHOST_BIN
export RHOST_BIN_ROOT := $(CURDIR)
export RHOST_REPO_ROOT := $(CURDIR)
.PHONY: build build-rust check rust-check test fmt portability build-reference test-conformance
build:
	cargo build --locked --bin rhost
build-rust: build
fmt:
	cargo fmt
test:
	cargo test --locked
check: rust-check portability
rust-check:
	cargo fmt --check
	cargo clippy --locked --all-targets -- -D warnings
	cargo test --locked
	@mkdir -p target/domain-check
	rustc --edition=2024 --crate-type lib --crate-name rhost_domain src/domain/mod.rs -D warnings --out-dir target/domain-check
portability:
	./scripts/check-portability.sh
build-reference:
	@mkdir -p bin
	cd archive/go-v3.1.0 && go build -trimpath -ldflags "-X github.com/starfield17/rhost/internal/buildinfo.Version=3.1.0 -X github.com/starfield17/rhost/internal/buildinfo.Commit=d9daaa9" -o ../../bin/rhost-go ./cmd/rhost
test-conformance:
	@test -n "$(RHOST_BIN)" || (echo "set RHOST_BIN to an explicit binary" >&2; exit 1)
	cd archive/conformance-v1 && RHOST_TEST_CONFORMANCE=1 go test ./conformance -run 'TestConformance|TestSchema|TestHarness|TestLiveCorpusIntegrity|TestHistoricalEvidenceLinks' -count=1 -timeout 5m

.PHONY: test-live-smoke
test-live-smoke:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=<user>@<host>" >&2; exit 1)
	@test -n "$(RHOST_BIN)" || (echo "set RHOST_BIN to an explicit binary" >&2; exit 1)
	cd archive/conformance-v1 && RHOST_TEST_LIVE=1 go test ./conformance -run '^TestSmokeLive$$' -v -count=1 -parallel 3 -timeout 5m

.PHONY: test-live
test-live:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=<user>@<host>" >&2; exit 1)
	@test -n "$(RHOST_BIN)" || (echo "set RHOST_BIN to an explicit binary" >&2; exit 1)
	cd archive/conformance-v1 && RHOST_TEST_LIVE=1 go test ./conformance -run 'TestLiveExec|TestLiveDoctor|TestLiveTransportReuse|TestLiveControlPath' -v -count=1 -parallel 3 -timeout 10m

.PHONY: test-live-session
test-live-session:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=<user>@<host>" >&2; exit 1)
	@test -n "$(RHOST_BIN)" || (echo "set RHOST_BIN to an explicit binary" >&2; exit 1)
	cd archive/conformance-v1 && RHOST_TEST_LIVE=1 go test ./conformance -run 'TestLiveSession' -v -count=1 -parallel 3 -timeout 10m

.PHONY: test-live-fs
test-live-fs:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=<user>@<host>" >&2; exit 1)
	@test -n "$(RHOST_BIN)" || (echo "set RHOST_BIN to an explicit binary" >&2; exit 1)
	cd archive/conformance-v1 && RHOST_TEST_LIVE=1 go test ./conformance -run 'TestLiveFs' -v -count=1 -parallel 3 -timeout 20m

.PHONY: test-live-tools
test-live-tools:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=<user>@<host>" >&2; exit 1)
	@test -n "$(RHOST_BIN)" || (echo "set RHOST_BIN to an explicit binary" >&2; exit 1)
	cd archive/conformance-v1 && RHOST_TEST_LIVE=1 go test ./conformance -run 'TestLiveTools|TestLiveTunnel' -v -count=1 -parallel 3 -timeout 20m

.PHONY: test-live-all
test-live-all:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=<user>@<host>" >&2; exit 1)
	@test -n "$(RHOST_BIN)" || (echo "set RHOST_BIN to an explicit binary" >&2; exit 1)
	cd archive/conformance-v1 && RHOST_TEST_LIVE=1 go test ./conformance -run '^TestLive' -v -count=1 -parallel 3 -timeout 20m
