//! Shared delivery evidence for bounded, recoverable observations.
use crate::{hosts, mcp::Mcp, remote_fs_support::create, support::Harness};
use serde_json::{Value, json};
#[test]
fn file_pages_recover_long_lines_and_pending_matches() -> Result<(), String> {
    let h = Harness::open("context-file-pages")?;
    hosts::local_host(&h)?;
    let text = "界".repeat(4000);
    let path = create(&h, "tree/long", text.as_bytes())?;
    let mut m = Mcp::open(&h)?;
    let mut params = json!({"path":path,"max_bytes":127});
    let mut all = String::new();
    let mut id = 2;
    loop {
        let result = m.call(id, "rhost_fs_read", json!({"host":"gpu","params":params}))?;
        id += 1;
        assert_eq!(result["ok"], true, "{result}");
        let data = &result["data"];
        all.push_str(data["content"].as_str().ok_or("content")?);
        assert!(data["content"].as_str().ok_or("content")?.len() <= 127);
        if data["coverage"]["more"] == false {
            break;
        }
        params = data["coverage"]["next_request"]["params"].clone();
        assert!(id < 200);
    }
    assert_eq!(all, text);
    create(
        &h,
        "tree/matches",
        b"needle one\nneedle two\nneedle three\n",
    )?;
    let mut params =
        json!({"root":h.path("tree"),"glob":"matches","pattern":"needle","max_matches":1});
    let mut lines = Vec::new();
    loop {
        let result = m.call(id, "rhost_fs_search", json!({"host":"gpu","params":params}))?;
        id += 1;
        assert_eq!(result["ok"], true, "{result}");
        for file in result["data"]["items"].as_array().ok_or("items")? {
            for row in file["matches"].as_array().ok_or("matches")? {
                lines.push(row["line"].as_u64().ok_or("line")?);
            }
        }
        if result["data"]["next_cursor"].is_null() {
            break;
        }
        params["cursor"] = result["data"]["next_cursor"].clone();
        let pending = &result["data"]["pending"];
        params["match_offset"] = pending.get("match_offset").cloned().unwrap_or(json!(0));
        if let Some(hash) = pending.get("if_hash") {
            params["if_hash"] = hash.clone();
        } else {
            params.as_object_mut().ok_or("params")?.remove("if_hash");
        }
        assert!(id < 210);
    }
    assert_eq!(lines, vec![1, 2, 3]);
    m.close()
}
#[test]
fn run_observation_reads_and_searches_have_recoverable_source_ranges() -> Result<(), String> {
    let h = Harness::open("context-run-logs")?;
    hosts::local_host(&h)?;
    let state = h.path("state").to_string_lossy().into_owned();
    let h = h.env("RHOST_REMOTE_STATE", &state);
    let mut m = Mcp::open(&h)?;
    let prep = m.call(2, "rhost_helper_prepare", json!({"host":"gpu","params":{}}))?;
    let generation = prep["data"]["generation"].clone();
    let expiry = prep["data"]["remote_time_unix"].as_u64().ok_or("clock")? + 3600;
    let key = format!("{expiry}-{}", "d".repeat(64));
    let started=m.call(3,"rhost_run_start",json!({"host":"gpu","params":{"generation":generation,"key":key,"expires_unix":expiry,"cwd":"/","program":{"argv":["/bin/sh","-c","printf 'alpha\nneedle one\nneedle two\nomega'; exit 7"]},"environment":{"inherit":false,"values":{}},"wait_ms":3000}}))?;
    assert_eq!(started["ok"], true, "{started}");
    let base = json!({"generation":generation,"run_id":key});
    let mut p = base.clone();
    p["wait_ms"] = json!(3000);
    let watch = m.call(4, "rhost_run_watch", json!({"host":"gpu","params":p}))?;
    assert_eq!(
        watch["data"]["run"]["completion"]["exit_code"], 7,
        "{watch}"
    );
    assert!(watch["data"]["stdout"].get("content").is_none());
    p = base.clone();
    p["stream"] = json!("stdout");
    p["since"] = json!(0);
    p["max_bytes"] = json!(4);
    p["format"] = json!("base64");
    let read = m.call(5, "rhost_run_read", json!({"host":"gpu","params":p}))?;
    assert_eq!(read["data"]["output"]["content"], "YWxwaA==", "{read}");
    assert_eq!(read["data"]["output"]["next"], 4);
    assert!(read["data"]["output"].get("content_base64").is_none());
    let mut p = base;
    p["stream"] = json!("stdout");
    p["pattern"] = json!("needle");
    p["max_bytes"] = json!(16);
    p["max_matches"] = json!(1);
    p["context_lines"] = json!(0);
    let mut count = 0;
    let mut id = 6;
    loop {
        let search = m.call(id, "rhost_run_search", json!({"host":"gpu","params":p}))?;
        assert_eq!(search["ok"], true, "{search}");
        count += search["data"]["matches"].as_array().ok_or("matches")?.len();
        if search["data"]["more"] == false {
            break;
        }
        p["since"] = search["data"]["next"].clone();
        p["until"] = search["data"]["until"].clone();
        id += 1;
        assert!(id < 30, "search did not progress");
    }
    assert_eq!(count, 2);
    m.close()
}
