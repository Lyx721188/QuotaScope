//! Keep status-line polling read-only, including before GUI migrations run.
use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn json_does_not_create_or_migrate_settings() {
    let id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("qs-json-{}-{id}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    let data = root.join("QuotaScope");
    let settings = data.join("settings.json");
    let run = || {
        let output = Command::new(env!("CARGO_BIN_EXE_quotascope"))
            .arg("--json")
            .env("APPDATA", &root)
            .env("LOCALAPPDATA", root.join("local"))
            .env("USERPROFILE", root.join("home"))
            .output()
            .unwrap();
        assert!(output.status.success());
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(json["generatedAt"].is_string());
        assert!(json["accounts"].is_array());
    };
    run();
    assert!(!data.exists());
    fs::create_dir_all(&data).unwrap();
    let original = br#"{"hasRun":true,"enabledAccounts":["minimaxCN#1"],"pinnedWindows":{"codex":"codex.primary"}}"#;
    fs::write(&settings, original).unwrap();
    let before = fs::metadata(&settings).unwrap().modified().unwrap();
    run();
    assert_eq!(fs::read(&settings).unwrap(), original);
    assert_eq!(fs::metadata(&settings).unwrap().modified().unwrap(), before);
    assert_eq!(fs::read_dir(data).unwrap().count(), 1);
}
