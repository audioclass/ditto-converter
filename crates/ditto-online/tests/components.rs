use ditto_online::components::{approved_manifest, verify_bytes, ArchiveKind, ComponentManager};
use std::sync::{atomic::AtomicBool, Arc};
#[test]
fn approved_tuple_is_fixed_and_unknown_unsigned_updates_fail_closed() {
    let manifest = approved_manifest("x86_64-unknown-linux-gnu").unwrap();
    assert_eq!(manifest.yt_dlp.version, "2026.08.19");
    assert_eq!(manifest.deno.version, "2.9.7");
    assert_eq!(manifest.yt_dlp.length, 40446224);
    assert_eq!(manifest.deno.length, 41596794);
    assert_eq!(manifest.deno.archive, ArchiveKind::Zip);
    assert!(approved_manifest("evil-platform").is_err());
    let t = tempfile::tempdir().unwrap();
    let manager = ComponentManager::new(t.path().to_path_buf());
    assert!(manager
        .accept_update_manifest(b"{\"sequence\":99}")
        .is_err());
    assert!(manager.status().unwrap().is_none());
    assert!(manager
        .install("unknown", &Arc::new(AtomicBool::new(false)), |_| {})
        .is_err());
    assert!(manager
        .install(
            "x86_64-unknown-linux-gnu",
            &Arc::new(AtomicBool::new(true)),
            |_| {}
        )
        .is_err());
}
#[test]
fn payload_verification_rejects_length_or_digest_mismatch() {
    let pin = approved_manifest("x86_64-unknown-linux-gnu")
        .unwrap()
        .yt_dlp;
    assert!(verify_bytes(b"malicious executable", &pin).is_err());
}
