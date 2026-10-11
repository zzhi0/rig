use std::process::Command;

fn rig() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rig"))
}

#[test]
fn root_help_exposes_the_commands() {
    let output = rig().arg("--help").output().unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("serve"));
    assert!(help.contains("run"));
    assert!(help.contains("--version"));
}

#[test]
fn run_help_explains_model_and_explicit_file_selection() {
    let output = rig().args(["run", "--help"]).output().unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("--model"));
    assert!(help.contains("--read"));
    assert!(help.contains("deepseek-flash"));
}

#[test]
fn run_without_credentials_fails_before_calling_the_model() {
    let output = rig()
        .args(["run", "hello"])
        .env_remove("DEEPSEEK_API_KEY")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("DEEPSEEK_API_KEY")
    );
}

#[test]
fn invalid_file_selection_fails_before_calling_the_model() {
    let directory = tempfile::tempdir().unwrap();
    let output = rig()
        .args(["run", "read my file", "--read"])
        .arg(directory.path().join("missing.txt"))
        .env("DEEPSEEK_API_KEY", "test-key")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("missing.txt"));
    assert!(!error.contains("Calling DeepSeek"));
    assert!(!error.contains("test-key"));
}

#[test]
fn version_uses_the_package_version() {
    let output = rig().arg("--version").output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("rig {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn serve_help_explains_port_and_asset_directory_defaults() {
    let output = rig().args(["serve", "--help"]).output().unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("7878"));
    assert!(help.contains("web/dist"));
    assert!(help.contains("current working directory"));
}

#[test]
fn invalid_ports_fail_before_starting_a_server() {
    let output = rig().args(["serve", "--port", "65536"]).output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr).unwrap().contains("--port"));
}
