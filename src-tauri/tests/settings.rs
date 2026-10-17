use ditto_app::settings::Settings;
#[test]
fn output_folder_roundtrip_and_corrupt_settings_recover() {
    let dir = std::env::current_dir()
        .unwrap()
        .join("../.work/settings-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("settings.json");
    let s = Settings {
        output_dir: Some("Music/Example Output".into()),
    };
    s.save(&path).unwrap();
    assert_eq!(Settings::load(&path).output_dir, s.output_dir);
    std::fs::write(&path, b"broken").unwrap();
    assert_eq!(Settings::load(&path).output_dir, None);
}
