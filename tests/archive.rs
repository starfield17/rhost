use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::Path};

fn collect(root: &Path, path: &Path, files: &mut BTreeMap<String, String>) -> std::io::Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect(root, &path, files)?;
        } else {
            let key = path
                .strip_prefix(root)
                .map_err(std::io::Error::other)?
                .to_string_lossy()
                .replace('\\', "/");
            files.insert(key, format!("{:x}", Sha256::digest(fs::read(path)?)));
        }
    }
    Ok(())
}

#[test]
fn archived_sources_and_versions_are_frozen() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("archive");
    let expected: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(root.join("MANIFEST.json"))?)?;
    let mut actual = BTreeMap::new();
    for name in ["go-v3.1.0", "conformance-v1"] {
        collect(&root, &root.join(name), &mut actual)?;
    }
    assert_eq!(
        actual, expected,
        "archive is immutable; do not regenerate the manifest"
    );
    assert_eq!(
        fs::read_to_string(root.join("go-v3.1.0/VERSION"))?.trim(),
        "3.1.0"
    );
    Ok(())
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
        let raw = fs::read(root.join(name))
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
