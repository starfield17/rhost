//! A v4 invocation must not discover or delete local state from the old root.

#[test]
fn v4_tunnel_listing_ignores_and_preserves_legacy_state() -> Result<(), Box<dyn std::error::Error>>
{
    let root = std::env::temp_dir().join(format!(
        "rhost-persistence-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let old = root.join("tunnels");
    std::fs::create_dir_all(&old)?;
    let id = "t_0123456789abcdef0123456789abcdef";
    let record = old.join(format!("{id}.json"));
    let body = format!(
        r#"{{"tunnel_id":"{id}","host":"example-host","kind":"socks","listen":"127.0.0.1:1234"}}"#
    );
    std::fs::write(&record, &body)?;

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rhost"))
        .args(["tunnel", "list", "--json"])
        .env("RHOST_STATE_DIR", &root)
        .output()?;
    assert!(output.status.success(), "{:?}", output.stderr);
    let answer: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(answer["data"]["tunnels"], serde_json::json!([]));
    assert_eq!(std::fs::read_to_string(&record)?, body);
    assert!(!root.join("v4/tunnels").join(format!("{id}.json")).exists());
    std::fs::remove_dir_all(root)?;
    Ok(())
}
