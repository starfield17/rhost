//! The Go legacy boundary must stay closed.
//!
//! The archived Go v3.1.0 implementation and its conformance harness were split
//! out to the read-only `rhost-go-old` repository at v4.4.1. These tests make
//! that a fact the build keeps, not a one-time cleanup: no Go build authority
//! may reappear here, and the two plugin manifests stay a checked mirror of the
//! single Rust version authority.

use std::path::Path;

#[test]
fn archived_go_history_stays_out_of_the_active_tree() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for name in ["archive", "go.mod", "go.sum", "VERSION", "internal", "cmd"] {
        assert!(
            !root.join(name).exists(),
            "archived Go history returned to the active tree: {name}"
        );
    }
    // The old Go snapshot is retained elsewhere, not vendored back in.
    assert!(
        !root.join("archive/MANIFEST.json").exists(),
        "the frozen Go manifest must not be re-imported"
    );
}

#[test]
fn active_tree_has_no_go_build_authority_and_plugin_versions_mirror_cargo() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for name in ["go.mod", "go.sum", "VERSION", "internal", "cmd"] {
        assert!(
            !root.join(name).exists(),
            "legacy entry point returned: {name}"
        );
    }
    for name in ["plugin.json", ".codex-plugin/plugin.json"] {
        let raw = std::fs::read(root.join(name))
            .unwrap_or_else(|error| panic!("read distribution manifest {name}: {error}"));
        let manifest: serde_json::Value = serde_json::from_slice(&raw)
            .unwrap_or_else(|error| panic!("parse distribution manifest {name}: {error}"));
        assert_eq!(
            manifest.get("version").and_then(serde_json::Value::as_str),
            Some(env!("CARGO_PKG_VERSION")),
            "{name} is a checked mirror of Cargo.toml, not a build authority"
        );
    }
}
