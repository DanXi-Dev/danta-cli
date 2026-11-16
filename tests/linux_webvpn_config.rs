#![cfg(target_os = "linux")]

use std::{fs, process::Command};

#[test]
fn cli_uses_xdg_config_instead_of_its_working_directory() {
    let root = std::env::temp_dir().join(format!("danta-config-{:016x}", rand::random::<u64>()));
    let config_home = root.join("config");
    let working_dir = root.join("workspace/target/release");
    let config = config_home.join("danta-cli");
    fs::create_dir_all(&working_dir).unwrap();
    fs::create_dir_all(&config).unwrap();
    fs::write(config.join("webvpn.json"), "saved session").unwrap();
    fs::write(working_dir.join("webvpn.json"), "unrelated file").unwrap();

    let result = Command::new(env!("CARGO_BIN_EXE_danta-cli"))
        .current_dir(&working_dir)
        .env("XDG_CONFIG_HOME", &config_home)
        .args(["webvpn", "logout"])
        .output()
        .unwrap();

    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!config.join("webvpn.json").exists());
    assert_eq!(
        fs::read_to_string(working_dir.join("webvpn.json")).unwrap(),
        "unrelated file"
    );
    fs::remove_dir_all(root).unwrap();
}
