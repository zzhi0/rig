use std::process::Command;

fn rig() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rig"))
}

#[test]
fn root_help_exposes_the_serve_command() {
    let output = rig().arg("--help").output().unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("serve"));
    assert!(help.contains("--version"));
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
