#[test]
fn windows_helpers_are_created_hidden_and_suspended_before_job_assignment() {
    let flags = ditto_core::process::WINDOWS_HELPER_FLAGS;
    assert_ne!(flags & 0x08000000, 0);
    assert_ne!(
        flags & 0x00000004,
        0,
        "a helper must not run before it belongs to its kill-on-close job"
    );
}
