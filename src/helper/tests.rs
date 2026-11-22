use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};
#[test]
fn symlinked_cached_executables_are_not_executed() {
    let root = std::env::temp_dir()
        .canonicalize()
        .unwrap_or_else(|e| panic!("temp: {e}"))
        .join(format!(
            "rhost-cache-{}",
            crate::random::hex(16).unwrap_or_else(|e| panic!("random: {e}"))
        ));
    let client = Client::new(crate::transport::Config::default());
    let helper = Helper::new(&client, "gpu");
    let cache = root.join(helper.cache.trim_start_matches("~/"));
    std::fs::create_dir_all(&cache).unwrap_or_else(|e| panic!("cache: {e}"));
    std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o700))
        .unwrap_or_else(|e| panic!("mode: {e}"));
    let target = root.join("untrusted");
    let marker = root.join("executed");
    std::fs::write(
        &target,
        format!(
            "#!/bin/sh\nprintf forbidden > {}\n",
            crate::shell::quote(&marker.to_string_lossy())
        ),
    )
    .unwrap_or_else(|e| panic!("source: {e}"));
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700))
        .unwrap_or_else(|e| panic!("mode: {e}"));
    symlink(&target, cache.join("rhost-helper")).unwrap_or_else(|e| panic!("link: {e}"));
    let result = std::process::Command::new("sh")
        .arg("-c")
        .arg(helper.command("probe\n"))
        .env("HOME", &root)
        .output()
        .unwrap_or_else(|e| panic!("guard: {e}"));
    assert!(!result.status.success());
    assert!(!marker.exists());
    std::fs::remove_dir_all(&root).unwrap_or_else(|e| panic!("cleanup: {e}"));
}

#[test]
fn explicit_abi_selects_userspace_architecture_and_requires_the_observed_os() -> Result<(), String>
{
    use target::Target;
    let abi = "linux-armv7-eabihf";
    let explicit = || Target::explicit(abi).map_err(|e| e.message);
    let matched = Target::observed("Linux\n", Some(explicit()?)).map_err(|e| e.message)?;
    assert_eq!(matched.abi(), abi);
    assert!(matches!(
        Target::observed("Darwin\n", Some(explicit()?)),
        Err(error) if error.code == "HELPER_INCOMPATIBLE"
    ));
    for invalid in [
        "linux-armv7",
        "darwin-armv7-eabihf",
        "linux-amd64",
        "linux-../../x86_64",
    ] {
        assert!(matches!(
            Target::explicit(invalid),
            Err(error) if error.code == "CONFIG_INVALID"
        ));
    }
    assert!(Target::observed("Linux\narmv7l\n", None).is_err());
    Ok(())
}

#[test]
fn pack_requires_matching_target_source_manifest_sidecar_and_bytes() -> Result<(), String> {
    use artifact::{Binary, from_pack};
    use serde_json::json;
    use target::Target;
    let root = std::env::temp_dir().join(format!(
        "rhost-pack-{}",
        crate::random::hex(16).map_err(|e| e.to_string())?
    ));
    let abi = "linux-armv7-eabihf";
    let target = Target::explicit(abi).map_err(|e| e.message)?;
    let source = source_digest();
    let bytes = b"pack image";
    let digest = Binary::bytes(bytes.to_vec()).map_err(|e| e.message)?.sha256;
    let directory = root.join(abi).join(&source).join(&digest);
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let manifest = |entries| {
        std::fs::write(
            root.join("manifest.json"),
            json!({"helpers": entries}).to_string(),
        )
        .map_err(|e| e.to_string())
    };
    let entry = |target: &str, source: &str| json!({"target":target,"source_digest":source,"binary_sha256":digest});
    let image = directory.join("rhost-helper");
    let sidecar = directory.join("rhost-helper.sha256");
    std::fs::write(&image, bytes).map_err(|e| e.to_string())?;
    std::fs::write(&sidecar, format!("{digest}  rhost-helper\n")).map_err(|e| e.to_string())?;
    manifest(vec![entry(abi, &source)])?;
    assert_eq!(
        from_pack(&root, &target).map_err(|e| e.message)?.bytes,
        bytes
    );
    for entries in [
        vec![entry("linux-aarch64", &source)],
        vec![entry(abi, &"0".repeat(64))],
        vec![entry(abi, &source), entry(abi, &source)],
    ] {
        manifest(entries)?;
        assert!(
            matches!(from_pack(&root, &target), Err(error) if error.code == "HELPER_INCOMPATIBLE")
        );
    }
    manifest(vec![entry(abi, &source)])?;
    std::fs::write(&sidecar, format!("{}  rhost-helper\n", "0".repeat(64)))
        .map_err(|e| e.to_string())?;
    assert!(matches!(from_pack(&root, &target), Err(error) if error.code == "HELPER_INCOMPATIBLE"));
    std::fs::write(&sidecar, format!("{digest}  rhost-helper\n")).map_err(|e| e.to_string())?;
    std::fs::write(&image, b"corrupt").map_err(|e| e.to_string())?;
    assert!(matches!(from_pack(&root, &target), Err(error) if error.code == "HELPER_INCOMPATIBLE"));
    std::fs::remove_dir_all(&root).map_err(|e| e.to_string())
}

#[test]
fn fixed_helper_requires_an_exact_remote_path_and_full_optional_pin() {
    use target::Fixed;
    let digest = "a".repeat(64);
    assert!(Fixed::new("~/tools/helper".into(), Some(digest)).is_ok());
    for path in [
        "rhost-helper",
        "~/tools/../helper",
        "/tools/./helper",
        "/tools/helper/",
    ] {
        assert!(Fixed::new(path.into(), None).is_err());
    }
    assert!(Fixed::new("~/tools/helper".into(), Some("bad".into())).is_err());
}
