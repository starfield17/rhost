//! The two documented file-input formats exercised through the real CLI.

use crate::hosts::{local_host, scp_stub};
use crate::support::{Harness, envelope, fail, python3_available, want_code};
use serde_json::Value;

fn documented_json(heading: &str) -> Result<Value, String> {
    let reference = include_str!("../../skills/rhost/references/CLI.md");
    let section = reference
        .split_once(heading)
        .ok_or_else(|| format!("CLI reference omits {heading}"))?
        .1
        .split("\n##")
        .next()
        .ok_or("missing example section")?;
    let body = section
        .split_once("```json\n")
        .and_then(|(_, body)| body.split_once("\n```"))
        .map(|(body, _)| body)
        .ok_or_else(|| format!("{heading} omits its JSON example"))?;
    serde_json::from_str(body).map_err(|error| fail("parse documented JSON", error))
}

#[test]
fn the_documented_patch_replaces_original_lines_and_refuses_a_stale_hash() -> Result<(), String> {
    let mut document = documented_json("### Patch document\n")?;
    assert_eq!(document["sha256"], "<sha256-from-read>");
    let harness = Harness::open("documented-patch")?;
    local_host(&harness)?;
    if !python3_available() {
        return Err("documented patch verification requires python3".into());
    }
    harness.write("original.txt", "alpha\nbeta\n", None)?;
    let target = harness.path("original.txt").to_string_lossy().into_owned();
    let (_, read) = envelope(&harness, &["fs", "read", "gpu", &target, "--json"])?;
    document["sha256"] = read["data"]["sha256"].clone();
    let patch = harness.path("patch.json");
    std::fs::write(&patch, document.to_string()).map_err(|error| fail("write patch", error))?;
    let args = [
        "fs",
        "patch",
        "gpu",
        &target,
        "--patch",
        &patch.to_string_lossy(),
        "--json",
    ];
    let (outcome, value) = envelope(&harness, &args)?;
    assert_eq!(outcome.status, 0, "{value}");
    assert_eq!(
        std::fs::read_to_string(&target).map_err(|error| fail("read patch result", error))?,
        "alpha\ngamma\n"
    );
    let (outcome, value) = envelope(&harness, &args)?;
    assert_eq!(outcome.status, 255, "{value}");
    want_code(&value, "FILE_CONFLICT")
}

#[test]
fn the_documented_batch_preserves_copy_direction_and_reports_each_item() -> Result<(), String> {
    let mut manifest = documented_json("### Batch manifest\n")?;
    let harness = Harness::open("documented-batch")?;
    local_host(&harness)?;
    harness.stub(
        "scp",
        &scp_stub("case \"$src\" in *:*) printf 'result\\n' > \"$dest\" ;; esac\nexit 0"),
    )?;
    harness.write("upload.txt", "upload\n", None)?;
    assert_eq!(manifest[0]["op"], "put");
    assert_eq!(manifest[1]["op"], "get");
    assert_eq!(manifest[0]["destination"], "~/work/upload.txt");
    assert_eq!(manifest[1]["source"], "~/work/result.txt");
    // Only local paths are relocated into scratch; remote operands stay literal.
    for (index, key) in [(0, "source"), (1, "destination")] {
        let path = manifest[index][key]
            .as_str()
            .ok_or("example has no local path")?;
        manifest[index][key] = Value::from(harness.path(path).to_string_lossy().into_owned());
    }
    let path = harness.path("copies.json");
    std::fs::write(&path, manifest.to_string()).map_err(|error| fail("write manifest", error))?;
    let (outcome, value) = envelope(
        &harness,
        &[
            "fs",
            "batch",
            "gpu",
            "--manifest",
            &path.to_string_lossy(),
            "--json",
        ],
    )?;
    assert_eq!(outcome.status, 0, "{value}");
    assert_eq!(value["data"]["succeeded"], 2);
    assert_eq!(value["data"]["failed"], 0);
    assert_eq!(value["data"]["items"][0]["op"], "put");
    assert_eq!(value["data"]["items"][1]["op"], "get");
    assert_eq!(
        std::fs::read_to_string(harness.path("download.txt"))
            .map_err(|error| fail("read download", error))?,
        "result\n"
    );
    Ok(())
}
