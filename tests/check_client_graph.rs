//! Fake-only checks of `scripts/check-client-graph`: `cargo` is a stub on
//! PATH that prints a scripted dependency tree and logs its arguments.
#![cfg(unix)]

use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

struct Run {
    status: i32,
    stdout: String,
    stderr: String,
    args: String,
}

/// `tree`: what `cargo tree` prints; `cargo_fails`: it exits non-zero.
fn run(tree: &str, cargo_fails: bool) -> Run {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let log = dir.path().join("args.log");
    let tree_file = dir.path().join("tree.txt");
    fs::write(&tree_file, tree).unwrap();
    let exit = if cargo_fails { "exit 101" } else { "exit 0" };
    let stub = bin.join("cargo");
    fs::write(
        &stub,
        format!(
            "#!/bin/sh\necho \"$*\" > '{}'\ncat '{}'\n{exit}\n",
            log.display(),
            tree_file.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/check-client-graph");
    let output = Command::new("bash")
        .arg(script)
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .output()
        .unwrap();
    Run {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into(),
        stderr: String::from_utf8_lossy(&output.stderr).into(),
        args: fs::read_to_string(log).unwrap_or_default(),
    }
}

const CLEAN: &str = "webex-tui v0.8.0 (/x)\nwebex-bridge v0.1.0 (/y)\nhyper v1.6.0\nhyperlocal v0.9.1\ntokio v1.45.0\n";

#[test]
fn a_clean_graph_passes_and_covers_all_targets_from_the_lockfile() {
    let run = run(CLEAN, false);
    assert_eq!(run.status, 0, "{}", run.stderr);
    assert!(run.stdout.contains("client-only graph OK (5 crates"));
    assert!(run.args.contains("--locked"), "{}", run.args);
    assert!(run.args.contains("--target all"), "{}", run.args);
    assert!(run.args.contains("-e normal"), "{}", run.args);
}

#[test]
fn any_forbidden_crate_fails() {
    for crate_name in [
        "webex",
        "oauth2",
        "reqwest",
        "tokio-tungstenite",
        "async-tungstenite",
        "rusqlite",
        "rustls",
        "tokio-rustls",
        "tokio-native-tls",
        "hyper-tls",
        "openssl-sys",
        "ring",
        "aws-lc-rs",
        "aws-lc-sys",
        "axum",
    ] {
        let run = run(&format!("{CLEAN}{crate_name} v1.0.0\n"), false);
        assert_eq!(run.status, 1, "{crate_name}: {}", run.stderr);
        assert!(run.stderr.contains(crate_name), "{crate_name}");
    }
}

#[test]
fn similar_names_are_not_forbidden() {
    // Exact names only: these are fine.
    let run = run(
        &format!("{CLEAN}webex-bridge-extra v1.0.0\nringbuf v0.4.0\n"),
        false,
    );
    assert_eq!(run.status, 0, "{}", run.stderr);
}

#[test]
fn a_failing_cargo_tree_fails_the_check() {
    let run = run(CLEAN, true);
    assert_eq!(run.status, 2, "{}", run.stderr);
    assert!(run.stderr.contains("cargo tree did not run"));
}
