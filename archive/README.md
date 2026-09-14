# Frozen Go reference

go-v3.1.0/ contains the exact tracked source bytes at commit d9daaa9 (v3.1.0),
including VERSION, plugin metadata, go.mod, go.sum, original tests, installer,
skills, documentation and release workflows. Those version authorities remain
3.1.0 permanently; they do not control the Rust binary.

conformance-v1/ is the independently extracted compatibility harness, finalized
and frozen at migration bootstrap. It accepts RHOST_BIN and validates v1/v2
observations without production imports. Go is optional reference tooling only.
Future acceptance tests are written in Rust; never repair this harness in place.

MANIFEST.json records SHA-256 for every file in both snapshots.
Build the legacy reference outside the archive with make build-reference.
Run its hermetic corpus with RHOST_BIN=./bin/rhost-go make test-conformance.
The owner reports the original Go binary passed real SSH verification; this
does not establish live verification of the extracted harness or Rust.
