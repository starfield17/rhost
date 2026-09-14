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
