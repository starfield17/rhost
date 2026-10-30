//! Public discovery and bounded namespace admission, using isolated real files.
use crate::support::{Harness, envelope_with_stdin};
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
fn request(h: &Harness, op: &str, params: Value) -> Result<Value, String> {
    let q = json!({"schema_version":4,"operation":op,"host":"gpu","params":params});
    envelope_with_stdin(h, &["request", "--from", "-"], q.to_string().as_bytes()).map(|(_, v)| v)
}
fn setup(name: &str) -> Result<(Harness, Value, u64), String> {
    let h = Harness::open(name)?;
    crate::hosts::local_host(&h)?;
    let state = h.path("remote-state").to_string_lossy().into_owned();
    let h = h.env("RHOST_REMOTE_STATE", &state);
    let prepared = request(&h, "helper.prepare", json!({}))?;
    assert_eq!(prepared["ok"], true, "{prepared}");
    let now = prepared["data"]["remote_time_unix"]
        .as_u64()
        .ok_or("clock")?;
    Ok((h, prepared["data"]["generation"].clone(), now))
}
#[test]
fn state_usage_and_prune_preview_preserve_running_work() -> Result<(), String> {
    let (h, generation, now) = setup("state-usage")?;
    let key = format!("{}-{}", now + 3600, "a".repeat(64));
    let started = request(
        &h,
        "run.start",
        json!({"generation":generation,"key":key,"expires_unix":now+3600,"cwd":h.path("home"),"program":{"argv":["/bin/sleep","30"]},"environment":{"inherit":false,"values":{}},"deadline_ms":3000,"wait_ms":100}),
    )?;
    assert_eq!(started["ok"], true, "{started}");
    let usage = request(&h, "state.usage", json!({"generation":generation}))?;
    assert_eq!(usage["data"]["complete"], true, "{usage}");
    assert_eq!(usage["data"]["records"], 1);
    assert!(usage["data"]["reserved_bytes"].as_u64().ok_or("reserved")? >= 145 * 1024 * 1024);
    let preview = request(
        &h,
        "state.prune",
        json!({"generation":generation,"kind":"runs","ids":[key]}),
    )?;
    assert_eq!(preview["data"]["dry_run"], true, "{preview}");
    assert_eq!(preview["data"]["items"][0]["eligible"], false);
    assert!(h.path(&format!("remote-state/runs/{key}")).is_dir());
    let stopped = request(
        &h,
        "run.signal",
        json!({"generation":generation,"run_id":key,"signal":"TERM"}),
    )?;
    assert_eq!(stopped["data"]["delivered"], true, "{stopped}");
    Ok(())
}
#[test]
fn run_discovery_pages_beyond_the_old_collection_ceiling() -> Result<(), String> {
    let (h, generation, now) = setup("state-pages")?;
    let key = format!("{}-{}", now + 3600, "a".repeat(64));
    let started = request(
        &h,
        "run.start",
        json!({"generation":generation,"key":key,"expires_unix":now+3600,"cwd":h.path("home"),"program":{"argv":["/bin/true"]},"environment":{"inherit":false,"values":{}},"wait_ms":1000}),
    )?;
    assert_eq!(started["data"]["run"]["status"], "exited", "{started}");
    let original = h.path(&format!("remote-state/runs/{key}"));
    for n in 1..4100 {
        let id = format!("{}-{n:064x}", now + 3600);
        let dir = h.path(&format!("remote-state/runs/{id}"));
        std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
        for entry in std::fs::read_dir(&original).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_type().map_err(|e| e.to_string())?.is_file() {
                std::fs::copy(entry.path(), dir.join(entry.file_name()))
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut params = json!({"generation":generation,"limit":256});
    loop {
        let page = request(&h, "run.list", params.clone())?;
        assert_eq!(page["ok"], true, "{page}");
        let items = page["data"]["items"].as_array().ok_or("items")?;
        assert!(items.len() <= 256);
        for item in items {
            assert!(seen.insert(item["id"].as_str().ok_or("id")?.to_string()));
        }
        if page["data"]["next_cursor"].is_null() {
            break;
        }
        params["cursor"] = page["data"]["next_cursor"].clone();
    }
    assert_eq!(seen.len(), 4100);
    let listed = request(&h, "fs.changeset.list", json!({"generation":generation}))?;
    assert_eq!(listed["data"]["items"], json!([]), "{listed}");
    Ok(())
}
#[test]
fn conservative_capacity_refusal_precedes_new_launch_registration() -> Result<(), String> {
    let (h, generation, now) = setup("state-capacity")?;
    let key = format!("{}-{}", now + 3600, "a".repeat(64));
    let started = request(
        &h,
        "run.start",
        json!({"generation":generation,"key":key,"expires_unix":now+3600,"cwd":h.path("home"),"program":{"argv":["/bin/sleep","10"]},"environment":{"inherit":false,"values":{}},"deadline_ms":1000,"wait_ms":100}),
    )?;
    assert_eq!(started["ok"], true, "{started}");
    let original = h.path(&format!("remote-state/runs/{key}"));
    for n in 1..64 {
        let dir = h.path(&format!("remote-state/runs/{}-{n:064x}", now + 3600));
        std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
        for name in ["status", "owner.lock"] {
            std::fs::copy(original.join(name), dir.join(name)).map_err(|e| e.to_string())?;
        }
    }
    let new_key = format!("{}-{}", now + 3600, "b".repeat(64));
    let refused = request(
        &h,
        "run.start",
        json!({"generation":generation,"key":new_key,"expires_unix":now+3600,"cwd":h.path("home"),"program":{"argv":["/bin/sh","-c","touch forbidden"]},"environment":{"inherit":false,"values":{}}}),
    )?;
    assert_eq!(
        refused["error"]["code"], "STATE_CAPACITY_EXCEEDED",
        "{refused}"
    );
    assert!(!h.path("home/forbidden").exists());
    assert!(!h.path(&format!("remote-state/runs/{new_key}")).exists());
    assert_eq!(
        request(&h, "state.usage", json!({"generation":generation}))?["ok"],
        true
    );
    Ok(())
}

#[test]
fn pending_records_and_partial_tombstones_have_explicit_prune_receipts() -> Result<(), String> {
    let (h, generation, now) = setup("state-prune-receipts")?;
    let base = h.path("remote-state/queries");
    std::fs::create_dir(&base).map_err(|e| e.to_string())?;
    std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| e.to_string())?;
    let good = format!("{}-{}", now - 1, "c".repeat(64));
    let bad = format!("{}-{}", now - 1, "d".repeat(64));
    let tomb = format!("{}-{}", now - 1, "e".repeat(64));
    for name in [
        format!(".pending-{good}"),
        format!(".pending-{bad}"),
        format!(".pruned-{tomb}"),
    ] {
        let path = base.join(name);
        std::fs::create_dir(&path).map_err(|e| e.to_string())?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    std::fs::write(base.join(format!(".pending-{bad}/foreign")), b"retain")
        .map_err(|e| e.to_string())?;
    std::fs::write(base.join(format!(".pruned-{tomb}/paths")), b"partial")
        .map_err(|e| e.to_string())?;
    let before = std::fs::read(h.path("remote-state/clock")).map_err(|e| e.to_string())?;
    let preview = request(
        &h,
        "state.prune",
        json!({"generation":generation,"kind":"queries","ids":[good,bad,tomb]}),
    )?;
    assert_eq!(preview["data"]["dry_run"], true, "{preview}");
    assert_eq!(
        std::fs::read(h.path("remote-state/clock")).map_err(|e| e.to_string())?,
        before
    );
    assert!(base.join(format!(".pending-{good}")).exists());
    let pruned = request(
        &h,
        "state.prune",
        json!({"generation":generation,"kind":"queries","ids":[good,bad,tomb],"dry_run":false}),
    )?;
    assert_eq!(pruned["ok"], true, "{pruned}");
    assert_eq!(pruned["data"]["failed"], 1, "{pruned}");
    assert_eq!(pruned["data"]["items"][0]["pruned"], true);
    assert_eq!(
        pruned["data"]["items"][1]["error"]["code"],
        "STATE_UNAVAILABLE"
    );
    assert_eq!(pruned["data"]["items"][2]["pruned"], true);
    assert!(base.join(format!(".pending-{bad}/foreign")).is_file());
    assert!(!base.join(format!(".pruned-{tomb}")).exists());
    Ok(())
}
