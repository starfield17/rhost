//! The two documented file-input formats exercised through the real CLI.

use crate::hosts::{local_host, scp_stub};
use crate::support::{Harness, envelope, fail, want_code};
use serde_json::Value;

fn documented_json(heading: &str) -> Result<Value, String> {
    let reference = if heading == "### Patch document\n" {
        include_str!("../../skills/rhost/references/FILES.md")
    } else {
        include_str!("../../skills/rhost/references/CLI.md")
    };
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

#[test]
fn the_documented_exact_text_patch_replaces_target_text() -> Result<(), String> {
    let reference = include_str!("../../skills/rhost/references/FILES.md");
    let section = reference
        .split_once("# Exact-text replacement patch")
        .ok_or("CLI reference omits exact-text replacement patch section")?
        .1;
    let body = section
        .split_once("<< 'EOF'\n")
        .and_then(|(_, rest)| rest.split_once("\nEOF"))
        .map(|(json_text, _)| json_text)
        .ok_or("cannot find exact-text replacement heredoc in CLI reference")?;
    let mut document: Value =
        serde_json::from_str(body).map_err(|error| fail("parse documented JSON", error))?;
    assert_eq!(document["sha256"], "<sha256-from-read>");
    let harness = Harness::open("documented-exact-patch")?;
    local_host(&harness)?;
    harness.write("original.txt", "line 1\nmodified line 2\n", None)?;
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
        "line 1\ngamma\n"
    );
    // Chain another edit using the returned hash, without an intervening read.
    document["sha256"] = value["data"]["sha256"].clone();
    document["edits"] = serde_json::json!([{"old": "gamma\n", "new": "delta\n"}]);
    std::fs::write(&patch, document.to_string())
        .map_err(|error| fail("write next patch", error))?;
    let (outcome, value) = envelope(&harness, &args)?;
    assert_eq!(outcome.status, 0, "{value}");
    assert_eq!(
        std::fs::read_to_string(&target).map_err(|error| fail("read chained result", error))?,
        "line 1\ndelta\n"
    );
    Ok(())
}

#[test]
fn session_command_file_submits_the_same_multiline_program_as_inline() -> Result<(), String> {
    let harness = Harness::open("session-command-file")?;
    harness.stub("ssh", "exit 0")?;
    let program = "printf '%s\\n' 'a b'\nexport VALUE='中文'\nprintf '%s' \"$VALUE\"\n";
    harness.write("program.sh", program, None)?;
    let path = harness.path("program.sh");
    let (_, file) = envelope(
        &harness,
        &[
            "session",
            "exec",
            "gpu",
            "work",
            "--command-file",
            path.to_str().ok_or("path")?,
            "--json",
        ],
    )?;
    want_code(&file, "REMOTE_EXECUTION_UNKNOWN")?;
    let (_, inline) = envelope(
        &harness,
        &[
            "session",
            "exec",
            "gpu",
            "work",
            "--command",
            program,
            "--json",
        ],
    )?;
    want_code(&inline, "REMOTE_EXECUTION_UNKNOWN")?;
    let calls = harness.tool_calls("ssh");
    assert_eq!(calls.len(), 2);
    let decode = |encoded: &str| -> Result<String, String> {
        let output = std::process::Command::new("/bin/sh")
            .args([
                "-c",
                &format!(
                    "printf '%s' {} | /usr/bin/base64 -d",
                    crate::support::shell_quote(encoded)
                ),
            ])
            .output()
            .map_err(|error| error.to_string())?;
        assert!(output.status.success());
        String::from_utf8(output.stdout).map_err(|error| error.to_string())
    };
    for call in &calls {
        let outer = call.args.last().ok_or("remote command")?;
        let encoded = outer
            .split_once("printf %s ")
            .ok_or("outer payload")?
            .1
            .split(' ')
            .next()
            .ok_or("outer base64")?;
        let helper = decode(encoded)?;
        let encoded = helper
            .split_once("cmd=$(printf '%s' '")
            .ok_or("pane payload")?
            .1
            .split('\'')
            .next()
            .ok_or("pane base64")?;
        let payload = decode(encoded)?;
        assert_eq!(
            payload
                .split_once("\ncommand printf")
                .ok_or("completion marker")?
                .0,
            format!("{{\n{program}")
        );
    }
    assert!(!harness.calls_text().contains(path.to_str().ok_or("path")?));
    assert!(calls.iter().all(|call| {
        call.args
            .last()
            .is_some_and(|arg| !arg.contains("--command-file"))
    }));
    Ok(())
}
