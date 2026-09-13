BINARY  := rhost
PKG     := github.com/starfield17/rhost/internal/buildinfo
VERSION ?= $(shell cat VERSION)
COMMIT  ?= $(shell git rev-parse --short HEAD 2>/dev/null || echo none)
DATE    ?= $(shell date -u +%Y-%m-%dT%H:%M:%SZ)

# Release assets carry the bare version: the tag is `v0.1.0-alpha.1` and the
# installer asks for `rhost_0.1.0-alpha.1_darwin_arm64`.
ARTIFACT_VERSION := $(VERSION:v%=%)

# The four targets that ship, named once. `make dist` builds them, the release
# workflow re-executes each one on a runner of that platform and architecture,
# and scripts/check-release-contract.sh fails when the three readers drift
# (docs/architecture/engineering.md).
RELEASE_TARGETS := darwin/amd64 darwin/arm64 linux/amd64 linux/arm64
DIST_DIR ?= dist

BASE_LDFLAGS  := -X $(PKG).Commit=$(COMMIT) -X $(PKG).BuildDate=$(DATE)
LDFLAGS       := -X $(PKG).Version=$(VERSION) $(BASE_LDFLAGS)
DIST_LDFLAGS  := -X $(PKG).Version=$(ARTIFACT_VERSION) $(BASE_LDFLAGS)

.PHONY: build dist test test-live-smoke test-live test-live-session test-live-fs test-live-tools test-live-all vet fmt fmtcheck check portability contract install-test clean

build:
	go build -trimpath -ldflags "$(LDFLAGS)" -o bin/$(BINARY) ./cmd/rhost

# The release artifacts, built the way CI builds them. One Go cross-compile
# covers every target: the runners that verify these files never rebuild them,
# they execute exactly these bytes. Pass DATE=<timestamp> to reproduce a
# published build byte for byte.
dist:
	@set -eu; \
	dist="$(DIST_DIR)"; \
	case "$$dist" in ''|.|..|/) echo "refusing unsafe DIST_DIR=$$dist" >&2; exit 2;; esac; \
	[ ! -L "$$dist" ] || { echo "refusing symlink DIST_DIR=$$dist" >&2; exit 2; }; \
	parent=$$(dirname "$$dist"); base=$$(basename "$$dist"); mkdir -p "$$parent"; \
	abs=$$(cd "$$parent" && pwd -P)/$$base; cwd=$$(pwd -P); \
	[ "$$abs" != "$$cwd" ] && [ "$$abs" != / ] && [ "$$abs" != "$${HOME:-/nonexistent}" ] || \
		{ echo "refusing broad DIST_DIR=$$abs" >&2; exit 2; }; \
	mkdir -p "$$dist"; \
	for item in "$$dist"/* "$$dist"/.[!.]* "$$dist"/..?*; do \
		[ -e "$$item" ] || continue; name=$$(basename "$$item"); \
		case "$$name" in \
			rhost_*_darwin_amd64|rhost_*_darwin_amd64.sha256|rhost_*_darwin_arm64|rhost_*_darwin_arm64.sha256|rhost_*_linux_amd64|rhost_*_linux_amd64.sha256|rhost_*_linux_arm64|rhost_*_linux_arm64.sha256) [ -f "$$item" ] || { echo "refusing non-file release artifact $$item" >&2; exit 2; };; \
			*) echo "refusing to remove unexpected DIST_DIR content: $$item" >&2; exit 2;; \
		esac; \
	done; \
	for item in "$$dist"/rhost_*_darwin_amd64 "$$dist"/rhost_*_darwin_amd64.sha256 "$$dist"/rhost_*_darwin_arm64 "$$dist"/rhost_*_darwin_arm64.sha256 "$$dist"/rhost_*_linux_amd64 "$$dist"/rhost_*_linux_amd64.sha256 "$$dist"/rhost_*_linux_arm64 "$$dist"/rhost_*_linux_arm64.sha256; do [ -e "$$item" ] && rm -- "$$item" || true; done; \
	for target in $(RELEASE_TARGETS); do \
		goos="$${target%%/*}"; goarch="$${target##*/}"; \
		out="rhost_$(ARTIFACT_VERSION)_$${goos}_$${goarch}"; \
		echo "$(DIST_DIR)/$$out"; \
		GOOS="$$goos" GOARCH="$$goarch" CGO_ENABLED=0 go build -trimpath \
			-ldflags "$(DIST_LDFLAGS)" -o "$(DIST_DIR)/$$out" ./cmd/rhost; \
		( cd "$(DIST_DIR)" && if command -v sha256sum >/dev/null 2>&1; then sha256sum "$$out" > "$$out.sha256"; else shasum -a 256 "$$out" > "$$out.sha256"; fi ); \
	done

test:
	go test ./...

# Live tests are opt-in and take their target from the environment, so nothing
# about a specific machine is recorded in this repository (AGENTS.md §1). They
# drive the built binary as a child process, so persistence is proven across real
# process exits rather than assumed (docs/architecture/engineering.md).
test-live-smoke:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=<user>@<host>" && exit 1)
	RHOST_TEST_LIVE=1 go test ./internal/app/ -run '^TestSmokeLive$$' -v -count=1 -parallel 3 -timeout 5m

test-live:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=<user>@<host>" && exit 1)
	RHOST_TEST_LIVE=1 go test ./internal/app/ -run 'TestLiveExec|TestLiveDoctor|TestLiveTransportReuse|TestLiveControlPath' -v -count=1 -parallel 3 -timeout 10m

test-live-session:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=<user>@<host>" && exit 1)
	RHOST_TEST_LIVE=1 go test ./internal/app/ -run TestLiveSession -v -count=1 -parallel 3 -timeout 10m

test-live-fs:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=<user>@<host>" && exit 1)
	RHOST_TEST_LIVE=1 go test ./internal/app/ -run 'TestLiveFs' -v -count=1 -parallel 3 -timeout 20m

# The tools suite (fs read/write/patch, verified transfer, tunnels, session
# recovery) is separate because it is the heaviest in remote
# round trips, and because its tunnel tests open real listeners on both sides.
test-live-tools:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=<user>@<host>" && exit 1)
	RHOST_TEST_LIVE=1 go test ./internal/app/ -run 'TestLiveTools|TestLiveTunnel' -v -count=1 -parallel 3 -timeout 20m

test-live-all:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=<user>@<host>" && exit 1)
	RHOST_TEST_LIVE=1 go test ./internal/app/ -run '^TestLive' -v -count=1 -parallel 3 -timeout 20m

vet:
	go vet ./...

fmt:
	gofmt -w .

# Full pre-commit gate. Portability and the release contract are hard checks,
# not suggestions.
check: fmtcheck vet test portability contract

fmtcheck:
	@out=$$(gofmt -l .); if [ -n "$$out" ]; then echo "gofmt needed:"; echo "$$out"; exit 1; fi

portability:
	./scripts/check-portability.sh

contract: install-test
	./scripts/check-release-contract.sh

install-test:
	./scripts/test-install.sh

clean:
	rm -rf bin
