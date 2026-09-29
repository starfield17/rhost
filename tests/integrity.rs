//! Exercise the integrity gate against commits in an isolated repository.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Result<Self, String> {
        let root = std::env::temp_dir().join(format!(
            "rhost-integrity-{label}-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        let fixture = Self { root };
        fixture.git(&["init", "-q"])?;
        fixture.git(&["config", "user.name", "Test"])?;
        fixture.git(&["config", "user.email", "test@example.invalid"])?;
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("scripts")
            .join("check-integrity.sh");
        let script = fixture.root.join("scripts/check-integrity.sh");
        std::fs::create_dir_all(script.parent().ok_or("script has no parent")?)
            .map_err(|error| error.to_string())?;
        std::fs::copy(source, script).map_err(|error| error.to_string())?;
        fixture.write(
            "Makefile",
            "check: rust-check\nrust-check:\n\tcargo test --locked\n",
        )?;
        Ok(fixture)
    }

    fn git(&self, args: &[&str]) -> Result<(), String> {
        let output = Command::new("git")
            .current_dir(&self.root)
            .args(args)
            .output()
            .map_err(|error| error.to_string())?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            ))
        }
    }

    fn write(&self, path: &str, body: &str) -> Result<(), String> {
        let path = self.root.join(path);
        std::fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)
            .map_err(|error| error.to_string())?;
        std::fs::write(path, body).map_err(|error| error.to_string())
    }

    fn remove(&self, path: &str) -> Result<(), String> {
        std::fs::remove_file(self.root.join(path)).map_err(|error| error.to_string())
    }

    fn commit(&self, paths: &[&str], message: &str) -> Result<(), String> {
        let mut add = vec!["add", "--"];
        add.extend_from_slice(paths);
        self.git(&add)?;
        self.git(&["commit", "-qm", message])
    }

    fn check_rejects(&self, detail: &str) -> Result<(), String> {
        let output: Output = Command::new("bash")
            .current_dir(&self.root)
            .args(["scripts/check-integrity.sh", "HEAD^"])
            .output()
            .map_err(|error| error.to_string())?;
        let report = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if output.status.success() || !report.contains(detail) {
            return Err(format!(
                "integrity gate did not reject {detail:?}: status={:?} {report}",
                output.status.code()
            ));
        }
        Ok(())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn additions_cannot_hide_deleted_tests_or_assertions() -> Result<(), String> {
    let deleted = Fixture::new("deleted-test")?;
    deleted.write("tests/one.rs", "#[test] fn one() { assert!(true); }\n")?;
    deleted.commit(
        &["Makefile", "scripts/check-integrity.sh", "tests/one.rs"],
        "baseline",
    )?;
    deleted.remove("tests/one.rs")?;
    deleted.write(
        "src/replacement.rs",
        "#[test] fn two() { assert!(true); }\n",
    )?;
    deleted.commit(&["tests/one.rs", "src/replacement.rs"], "remove test")?;
    deleted.check_rejects("test file")?;

    let assertions = Fixture::new("moved-assertion")?;
    assertions.write("tests/one.rs", "#[test] fn one() { assert!(true); }\n")?;
    assertions.write("tests/two.rs", "#[test] fn two() { assert!(true); }\n")?;
    assertions.commit(
        &[
            "Makefile",
            "scripts/check-integrity.sh",
            "tests/one.rs",
            "tests/two.rs",
        ],
        "baseline",
    )?;
    assertions.write("tests/one.rs", "#[test] fn one() {}\n")?;
    assertions.write(
        "tests/two.rs",
        "#[test] fn two() { assert!(true); assert!(true); }\n",
    )?;
    assertions.commit(&["tests/one.rs", "tests/two.rs"], "move assertion")?;
    assertions.check_rejects("assertion")
}

#[test]
fn duplicate_gate_invocations_cannot_be_dropped() -> Result<(), String> {
    let fixture = Fixture::new("duplicate-gate")?;
    fixture.write(
        "Makefile",
        "check: rust-check\nrust-check:\n\tcargo test --locked\n\tcargo test --locked\n",
    )?;
    fixture.commit(&["Makefile", "scripts/check-integrity.sh"], "baseline")?;
    fixture.write(
        "Makefile",
        "check: rust-check\nrust-check:\n\tcargo test --locked\n",
    )?;
    fixture.commit(&["Makefile"], "drop duplicate gate")?;
    fixture.check_rejects("gate invocation")
}
