//! Offline audio conversion and bounded child processes.
pub mod audio;
pub mod process;
mod staging;
pub use audio::{AudioInfo, Engine};
pub use staging::recover_staging;
pub type Cancellation = std::sync::Arc<std::sync::atomic::AtomicBool>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error("operation cancelled")]
    Cancelled,
    #[error("helper failed: {0}")]
    Helper(String),
    #[error(transparent)]
    Io(std::io::Error),
    #[error("invalid probe data: {0}")]
    Json(#[from] serde_json::Error),
}
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        if error.kind() == std::io::ErrorKind::Interrupted {
            Self::Cancelled
        } else {
            Self::Io(error)
        }
    }
}
pub type Result<T> = std::result::Result<T, Error>;
pub(crate) fn check_cancel(cancel: &Cancellation) -> Result<()> {
    if cancel.load(std::sync::atomic::Ordering::Acquire) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}
