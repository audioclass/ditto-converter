use ditto_app::component_recovery::quarantine_failed_setup;
#[test]
fn repair_moves_only_failed_optional_setup_and_preserves_offline_files() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(".work/component-repair-test");
    std::fs::create_dir_all(&root).unwrap();
    let components = root.join("components");
    if components.exists() {
        std::fs::remove_dir_all(&components).unwrap();
    }
    std::fs::create_dir_all(&components).unwrap();
    std::fs::write(components.join("install.lock"), b"stale").unwrap();
    std::fs::write(root.join("offline.wav"), b"original").unwrap();
    assert!(quarantine_failed_setup(&components, true).is_err());
    quarantine_failed_setup(&components, false).unwrap();
    assert!(!components.exists());
    assert!(root.join("components.quarantine/install.lock").is_file());
    assert_eq!(
        std::fs::read(root.join("offline.wav")).unwrap(),
        b"original"
    );
}
#[cfg(unix)]
#[test]
fn repair_refuses_symlink_component_root() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(".work/component-repair-symlink");
    std::fs::create_dir_all(&root).unwrap();
    let victim = root.join("victim");
    std::fs::create_dir_all(&victim).unwrap();
    let components = root.join("components");
    if !components.exists() {
        std::os::unix::fs::symlink(&victim, &components).unwrap();
    }
    assert!(quarantine_failed_setup(&components, false).is_err());
    assert!(victim.is_dir());
}
