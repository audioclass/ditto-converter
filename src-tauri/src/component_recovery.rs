use std::path::Path;
/// Called only after explicit repair consent and while the app owns its instance lock.
/// Preserve failed setup for inspection; never make unverified bytes active.
pub fn quarantine_failed_setup(root: &Path, verified_active: bool) -> Result<(), String> {
    if verified_active {
        return Err("A verified online tuple is active; it will not be discarded by repair".into());
    }
    if root.file_name().and_then(|n| n.to_str()) != Some("components") {
        return Err("Unexpected optional-component directory".into());
    }
    let metadata = match std::fs::symlink_metadata(root) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.to_string()),
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("Component directory must not be a symlink".into());
    }
    let quarantine = root.with_file_name("components.quarantine");
    if let Ok(m) = std::fs::symlink_metadata(&quarantine) {
        if !m.is_dir() || m.file_type().is_symlink() {
            return Err("Recovery directory is not a regular directory".into());
        }
        std::fs::remove_dir_all(&quarantine).map_err(|e| e.to_string())?;
    }
    std::fs::rename(root, quarantine).map_err(|e| e.to_string())
}
