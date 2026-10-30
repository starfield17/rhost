//! Recovery advice comes from phase evidence, never diagnostic text.
use crate::support::{Harness, envelope_with_stdin};
use serde_json::{Value, json};
fn ask(h: &Harness, op: &str, params: Value) -> Result<Value, String> {
    let q = json!({"schema_version":4,"operation":op,"host":"gpu","params":params});
    envelope_with_stdin(h, &["request", "--from", "-"], q.to_string().as_bytes()).map(|(_, v)| v)
}
#[test]
fn preparation_and_submitted_disconnects_have_different_recovery() -> Result<(), String> {
    let h = Harness::open("recovery-phases")?;
    crate::hosts::local_host(&h)?;
    let state = h.path("remote-state").to_string_lossy().into_owned();
    let h = h.env("RHOST_REMOTE_STATE", &state);
    let missing = h.path("missing-helper").to_string_lossy().into_owned();
    let binary = std::env::var("RHOST_HELPER_BIN").map_err(|e| e.to_string())?;
    let h = h.env("RHOST_HELPER_BIN", &missing);
    let failed = ask(
        &h,
        "fs.write",
        json!({"path":h.path("never"),"content":"x"}),
    )?;
    assert_eq!(
        failed["error"]["recovery"],
        json!({"stage":"preparation","submission":"not_submitted","action":"retry_request"}),
        "{failed}"
    );
    assert!(!h.path("never").exists());
    let h = h.env("RHOST_HELPER_BIN", &binary);
    let p = ask(&h, "helper.prepare", json!({}))?;
    assert_eq!(p["ok"], true, "{p}");
    let g = &p["data"]["generation"];
    let expires = p["data"]["remote_time_unix"].as_u64().ok_or("clock")? + 3600;
    let key = format!("{expires}-{}", "f".repeat(64));
    let params = json!({"generation":g,"key":key,"expires_unix":expires,"cwd":h.path("home"),"program":{"argv":["/bin/sh","-c","printf x >> once"]},"environment":{"inherit":false,"values":{}},"wait_ms":1000});
    h.stub("ssh","for arg do last=$arg; done\ncase \"$last\" in *RHOST_FRAME_2_*) /bin/sh -c \"$last\" >/dev/null; exit 255;; *) exec /bin/sh -c \"$last\";; esac\n",)?;
    let unknown = ask(&h, "run.start", params.clone())?;
    assert_eq!(
        unknown["error"]["recovery"],
        json!({"stage":"submission","submission":"may_have_submitted","action":"retry_same_request"}),
        "{unknown}"
    );
    crate::hosts::local_host(&h)?;
    let retried = ask(&h, "run.start", params)?;
    assert_eq!(retried["data"]["run"]["status"], "exited", "{retried}");
    assert_eq!(
        std::fs::read(h.path("home/once")).map_err(|e| e.to_string())?,
        b"x"
    );
    Ok(())
}
