//! End-to-end tests: they run the real binary and assert on exit codes,
//! stdout and stderr — the contract a user actually depends on.

use std::path::Path;
use std::process::Command;

use assert_cmd::prelude::*;
use assert_fs::TempDir;
use predicates::prelude::*;

/// The binary under test, with a scratch home directory so the developer's own
/// `~/.ruststruct.json` can never leak into a test run.
///
/// Both variables are set because `std::env::home_dir` reads `HOME` on Unix and
/// `USERPROFILE` on Windows.
fn cmd(home: &Path) -> Command {
    let mut cmd = Command::cargo_bin("ruststruct").expect("binary is built by cargo test");
    cmd.env("HOME", home).env("USERPROFILE", home);
    cmd
}

#[test]
fn creates_a_working_project() {
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let root = work.path().join("myapp");

    cmd(home.path())
        .arg(&root)
        .assert()
        .success()
        .stdout(predicate::str::contains("created successfully"));

    assert!(root.join("Cargo.toml").is_file());
    assert!(root.join("src/lib.rs").is_file());

    // The generated project must build and pass its own tests.
    Command::new(env!("CARGO"))
        .args(["test", "--quiet"])
        .current_dir(&root)
        .assert()
        .success();
}

#[test]
fn dry_run_prints_a_plan_and_creates_nothing() {
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let root = work.path().join("dryapp");

    cmd(home.path())
        .args(["--dry-run", "--git"])
        .arg(&root)
        .assert()
        .success()
        .stdout(
            predicate::str::contains("package name : dryapp")
                .and(predicate::str::contains("cargo init --name dryapp"))
                .and(predicate::str::contains("git init"))
                .and(predicate::str::contains("created successfully").not()),
        );

    assert!(!root.exists());
}

#[test]
fn existing_directory_is_refused() {
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();

    cmd(home.path())
        .arg(work.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn missing_argument_shows_usage() {
    let home = TempDir::new().unwrap();

    cmd(home.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Usage: ruststruct"));
}

#[test]
fn help_and_version_are_available() {
    let home = TempDir::new().unwrap();

    cmd(home.path()).arg("--help").assert().success().stdout(
        predicate::str::contains("Scaffold a standard Rust project layout")
            .and(predicate::str::contains("~/.ruststruct.json")),
    );

    cmd(home.path())
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn user_config_in_home_is_applied() {
    let home = TempDir::new().unwrap();
    std::fs::write(
        home.path().join(".ruststruct.json"),
        r#"{ "dirs": ["scripts"], "files": { "rustfmt.toml": "edition = \"2024\"\n" } }"#,
    )
    .unwrap();
    let work = TempDir::new().unwrap();

    // --dry-run keeps this focused on config loading, with no `cargo init`.
    cmd(home.path())
        .arg("--dry-run")
        .arg(work.path().join("cfgapp"))
        .assert()
        .success()
        .stdout(predicate::str::contains("scripts/").and(predicate::str::contains("rustfmt.toml")));
}

#[test]
fn malformed_user_config_is_reported_with_its_path() {
    let home = TempDir::new().unwrap();
    std::fs::write(home.path().join(".ruststruct.json"), "{ not json").unwrap();
    let work = TempDir::new().unwrap();

    cmd(home.path())
        .arg(work.path().join("app"))
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("parsing config")
                .and(predicate::str::contains(".ruststruct.json")),
        );
}

#[test]
fn user_config_cannot_write_outside_the_project() {
    let home = TempDir::new().unwrap();
    std::fs::write(
        home.path().join(".ruststruct.json"),
        r#"{ "files": { "../escaped.txt": "pwned" } }"#,
    )
    .unwrap();
    let work = TempDir::new().unwrap();
    let root = work.path().join("guarded");

    cmd(home.path())
        .arg(&root)
        .assert()
        .failure()
        .stderr(predicate::str::contains("not a plain relative path"));

    assert!(!root.exists());
    assert!(!work.path().join("escaped.txt").exists());
}
