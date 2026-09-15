use std::process::Command;

#[test]
fn binary_reports_its_own_version_and_schema() -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new(env!("CARGO_BIN_EXE_rhost"))
        .args(["version", "--json"])
        .output()?;
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(value["schema_version"], 2);
    assert_eq!(value["operation"], "version");
    assert!(String::from_utf8(output.stdout)?.contains(env!("CARGO_PKG_VERSION")));
    Ok(())
}

/// `--help` is the authoritative inventory: each command's page must list the
/// flags its parser accepts, including the ones that never fit in a usage line
/// (`exec --cwd`, `exec --max-output-bytes`). The router once sent every leaf to
/// the root page, so this drives the real argv path rather than the renderer.
#[test]
fn help_lists_the_flags_each_command_accepts() -> Result<(), Box<dyn std::error::Error>> {
    for (argv, flags) in [
        (
            vec!["exec", "--help"],
            vec![
                "--command",
                "--command-file",
                "--cwd",
                "--env",
                "--timeout",
                "--max-output-bytes",
                "--fresh",
                "--stream",
            ],
        ),
        (vec!["doctor", "--help"], vec!["--timeout", "--fresh"]),
        (vec!["audit", "--help"], vec!["--limit", "--host"]),
        (
            vec!["session", "send", "--help"],
            vec!["--data", "--key", "--enter"],
        ),
        (
            vec!["fs", "write", "--help"],
            vec!["--from", "--if-hash", "--mode", "--parents"],
        ),
        (
            vec!["tunnel", "open", "--help"],
            vec!["--kind", "--listen", "--destination", "--allow-exposure"],
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rhost"))
            .args(&argv)
            .output()?;
        assert!(
            output.status.success(),
            "{argv:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let help = String::from_utf8(output.stdout)?;
        assert!(
            help.contains("Flags:"),
            "{argv:?} has no flag list:\n{help}"
        );
        for flag in flags {
            assert!(help.contains(flag), "{argv:?} omits {flag}:\n{help}");
        }
    }
    Ok(())
}

/// `--help` must name the concrete defaults a caller needs, not merely that a
/// default exists, and the file flags the CLI reference documents (FS-003/FS-004).
#[test]
fn help_names_concrete_defaults_and_file_flags() -> Result<(), Box<dyn std::error::Error>> {
    for (argv, wanted) in [
        (
            vec!["fs", "read", "--help"],
            vec!["256 KiB", "8 MiB", "--max-bytes"],
        ),
        (
            vec!["fs", "write", "--help"],
            vec!["0600", "--mode", "--if-hash"],
        ),
        (vec!["fs", "put", "--help"], vec!["5-minute"]),
        (
            vec!["session", "create", "--help"],
            vec!["60-second", "--cwd"],
        ),
        (vec!["session", "exec", "--help"], vec!["default"]),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rhost"))
            .args(&argv)
            .output()?;
        assert!(output.status.success(), "{argv:?}");
        let help = String::from_utf8(output.stdout)?;
        for needle in wanted {
            assert!(help.contains(needle), "{argv:?} omits {needle:?}:\n{help}");
        }
    }
    Ok(())
}

#[test]
fn release_waits_for_four_native_platforms_before_publication() {
    let workflow = include_str!("../.github/workflows/release.yml");
    for platform in ["darwin_amd64", "darwin_arm64", "linux_amd64", "linux_arm64"] {
        assert!(workflow.contains(&format!("platform: {platform}")));
    }
    assert_eq!(workflow.matches("platform: ").count(), 4);
    assert!(workflow.contains("\"$artifact\" version --json"));
    assert!(workflow.contains("\"$artifact\" --help"));
    assert!(workflow.contains("workflow_dispatch:"));
    assert!(workflow.contains("tags:"));
    assert!(workflow.contains("needs: native"));
    assert!(workflow.contains("if: startsWith(github.ref, 'refs/tags/v')"));
    assert!(workflow.contains("contents: write"));
    assert!(workflow.contains("gh release create \"v${version}\" dist/* --verify-tag"));
}
