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
