BINARY  := rhost
PKG     := github.com/starfield17/rhost/internal/buildinfo
VERSION ?= $(shell git describe --tags --always --dirty 2>/dev/null || echo dev)
COMMIT  ?= $(shell git rev-parse --short HEAD 2>/dev/null || echo none)
DATE    ?= $(shell date -u +%Y-%m-%dT%H:%M:%SZ)
LDFLAGS := -X $(PKG).Version=$(VERSION) -X $(PKG).Commit=$(COMMIT) -X $(PKG).BuildDate=$(DATE)

.PHONY: build test test-live test-live-session test-live-jobs test-live-all vet fmt check portability clean

build:
	go build -trimpath -ldflags "$(LDFLAGS)" -o bin/$(BINARY) ./cmd/rhost

test:
	go test ./...

# Live tests are opt-in and take their target from the environment, so nothing
# about a specific machine is recorded in this repository (AGENTS.md §1). They
# drive the built binary as a child process, so persistence is proven across real
# process exits rather than assumed (docs/ARCHITECTURE.md §41).
test-live:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=<user>@<host>" && exit 1)
	RHOST_TEST_LIVE=1 go test ./internal/app/ -run 'TestLiveExec|TestLiveDoctor|TestLiveTransportReuse|TestLiveControlPath' -v -timeout 10m

test-live-session:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=<user>@<host>" && exit 1)
	RHOST_TEST_LIVE=1 go test ./internal/app/ -run TestLiveSession -v -timeout 10m

test-live-jobs:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=<user>@<host>" && exit 1)
	RHOST_TEST_LIVE=1 go test ./internal/app/ -run 'TestLiveJob' -v -timeout 20m

test-live-all:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=<user>@<host>" && exit 1)
	RHOST_TEST_LIVE=1 go test ./internal/app/ -run 'TestLive' -v -timeout 20m

vet:
	go vet ./...

fmt:
	gofmt -w .

# Full pre-commit gate. Portability is a hard check, not a suggestion.
check: fmtcheck vet test portability

fmtcheck:
	@out=$$(gofmt -l .); if [ -n "$$out" ]; then echo "gofmt needed:"; echo "$$out"; exit 1; fi

portability:
	./scripts/check-portability.sh

clean:
	rm -rf bin
