//! Fake-only checks of `scripts/sign-tui-dev`: `cargo`, `security` and
//! `codesign` are stubs on PATH, so no real build, Keychain or signing
//! happens. The stubs log their arguments for assertions.
#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    /// `identities`: what `security find-identity` prints; `adhoc`: whether
    /// `codesign -d` reports an ad-hoc signature.
    fn new(identities: &str, adhoc: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let log = dir.path().join("calls.log");
        let target = dir.path().join("target");
        stub(
            &bin,
            "cargo",
            &format!(
                "echo \"cargo $*\" >> '{log}'\nmkdir -p '{target}/debug'\n: > '{target}/debug/webex-tui'\n",
                log = log.display(),
                target = target.display()
            ),
        );
        stub(
            &bin,
            "security",
            &format!(
                "echo \"security $*\" >> '{log}'\nprintf '%s\\n' '{identities}'\n",
                log = log.display()
            ),
        );
        let signature = if adhoc {
            "Signature=adhoc"
        } else {
            "Authority=Stub"
        };
        stub(
            &bin,
            "codesign",
            &format!(
                "echo \"codesign $*\" >> '{log}'\nif [ \"$1\" = -d ]; then printf 'Identifier=com.sgrimee.webex-tui\\n{signature}\\n' >&2; fi\n",
                log = log.display()
            ),
        );
        Self { dir }
    }

    fn run(&self, identity: Option<&str>) -> Output {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/sign-tui-dev");
        let mut command = Command::new("bash");
        command
            .arg(script)
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.dir.path().join("bin").display()),
            )
            .env("CARGO_TARGET_DIR", self.dir.path().join("target"))
            .env_remove("WEBEX_TUI_CODESIGN_IDENTITY");
        if let Some(identity) = identity {
            command.env("WEBEX_TUI_CODESIGN_IDENTITY", identity);
        }
        command.output().unwrap()
    }

    fn calls(&self) -> String {
        fs::read_to_string(self.dir.path().join("calls.log")).unwrap_or_default()
    }

    fn binary(&self) -> PathBuf {
        self.dir.path().join("target/debug/webex-tui")
    }
}

fn stub(bin: &Path, name: &str, body: &str) {
    let path = bin.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn signs_with_the_first_identity_and_the_stable_identifier_then_verifies() {
    let env = Env::new(
        "  1) ABCDEF \"Apple Development: Synthetic (TEAM)\"\n     1 valid identities found",
        false,
    );
    let output = env.run(None);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let calls = env.calls();
    let binary = env.binary();
    assert!(calls.contains("cargo build --bin webex-tui"));
    assert!(calls.contains(&format!(
        "codesign --force --sign Apple Development: Synthetic (TEAM) --identifier com.sgrimee.webex-tui --timestamp=none {}",
        binary.display()
    )));
    assert!(calls.contains(&format!("codesign --verify --strict {}", binary.display())));
    assert!(String::from_utf8_lossy(&output.stdout).contains("as com.sgrimee.webex-tui"));
}

#[test]
fn an_explicit_identity_skips_the_keychain_lookup() {
    let env = Env::new("", false);
    let output = env.run(Some("Webex TUI Development"));
    assert!(output.status.success());
    let calls = env.calls();
    assert!(!calls.contains("security"));
    assert!(calls.contains("--sign Webex TUI Development"));
}

#[test]
fn no_identity_or_an_ad_hoc_request_refuses_before_building() {
    for identity in [None, Some("-")] {
        let env = Env::new("     0 valid identities found", false);
        let output = env.run(identity);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("no ad-hoc"));
        let calls = env.calls();
        assert!(!calls.contains("cargo"), "{identity:?}");
        assert!(!calls.contains("codesign --"), "{identity:?}");
    }
}

#[test]
fn an_ad_hoc_result_is_refused() {
    let env = Env::new("", true);
    let output = env.run(Some("Webex TUI Development"));
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not persistently signed"));
}
