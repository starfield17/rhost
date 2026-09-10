BINARY  := rhost
PKG     := github.com/starfield17/rhost/internal/buildinfo
VERSION ?= $(shell git describe --tags --always --dirty 2>/dev/null || echo dev)
COMMIT  ?= $(shell git rev-parse --short HEAD 2>/dev/null || echo none)
DATE    ?= $(shell date -u +%Y-%m-%dT%H:%M:%SZ)
LDFLAGS := -X $(PKG).Version=$(VERSION) -X $(PKG).Commit=$(COMMIT) -X $(PKG).BuildDate=$(DATE)

.PHONY: build test test-live vet fmt clean

build:
	go build -trimpath -ldflags "$(LDFLAGS)" -o bin/$(BINARY) ./cmd/rhost

test:
	go test ./...

test-live:
	@test -n "$(RHOST_TEST_HOST)" || (echo "set RHOST_TEST_HOST=user@host" && exit 1)
	RHOST_TEST_LIVE=1 go test ./internal/app/ -run TestLiveExec -v

vet:
	go vet ./...

fmt:
	gofmt -w .

clean:
	rm -rf bin
