use quotascope_core::settings::AppSettings;

#[test]
fn old_settings_keep_windows_appearance_and_pulse_round_trips() {
    let old: AppSettings = serde_json::from_str(r#"{"panelSize":"large"}"#).unwrap();
    assert_eq!(old.panel_appearance, "windows");
    assert!(!old.pulse_glass);
    let pulse = AppSettings {
        panel_appearance: "pulse".into(),
        pulse_glass: true,
        ..old
    };
    let encoded = serde_json::to_string(&pulse).unwrap();
    let decoded: AppSettings = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded.panel_appearance, "pulse");
    assert!(decoded.pulse_glass);
    assert_eq!(decoded.panel_size, "large");
}
