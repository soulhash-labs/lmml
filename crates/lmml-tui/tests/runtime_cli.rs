use std::process::Command;

#[test]
fn runtime_read_only_commands_do_not_create_state_file() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let state_path = tempdir.path().join("lmml").join("state.toml");

    let status = Command::new(env!("CARGO_BIN_EXE_lmml"))
        .env("XDG_CONFIG_HOME", tempdir.path())
        .args(["runtime", "status"])
        .status()
        .expect("run runtime status");

    assert!(status.success());
    assert!(!state_path.exists());

    let output = Command::new(env!("CARGO_BIN_EXE_lmml"))
        .env("XDG_CONFIG_HOME", tempdir.path())
        .args(["runtime", "print-config", "opencode"])
        .output()
        .expect("run runtime print-config");

    assert!(output.status.success());
    assert!(!state_path.exists());
}
