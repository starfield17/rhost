# Frozen reference boundary

Do not edit go-v3.1.0/, conformance-v1/, or MANIFEST.json.
The snapshot's nested instructions are historical documents, not authorization
to develop Go. All future implementation and tests belong to the active Rust tree.
Build outputs must go outside archive/. Never tidy or update these Go modules.
make check verifies every frozen file; CI also rejects changes to the archive
against the base commit once the initial freeze has landed.
