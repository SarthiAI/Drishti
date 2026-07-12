//! Typed errors. No `Box<dyn Error>` in the public surface.

use std::path::PathBuf;

/// Errors raised while resolving and fetching a model artifact. Lives in core
/// (not in `drishti-models`) so the `ModelSource` trait signature stays here and
/// core does not depend on the loader crate. See ADR-003, ADR-004.
#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("model artifact not found: {id} (looked at {location})")]
    NotFound { id: String, location: String },

    #[error("integrity check failed for {id}: expected sha256 {expected}, got {actual}")]
    IntegrityCheckFailed {
        id: String,
        expected: String,
        actual: String,
    },

    #[error("download failed for {id} from {location}: {source}")]
    DownloadFailed {
        id: String,
        location: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("i/o error handling {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("model source misconfigured: {0}")]
    Config(String),
}

/// The top-level error every public check method returns.
#[derive(Debug, thiserror::Error)]
pub enum DrishtiError {
    #[error("model loading failed: {0}")]
    ModelLoadFailed(#[from] ModelError),

    #[error("inference failed: {0}")]
    InferenceFailed(String),

    #[error("tokenization failed: {0}")]
    TokenizationFailed(String),

    #[error("input too long: {len} tokens exceeds the configured maximum {max}")]
    InputTooLong { len: usize, max: usize },

    #[error("invalid configuration: {0}")]
    InvalidConfiguration(String),

    #[error("check '{0}' is not enabled in the configuration")]
    CheckNotEnabled(&'static str),
}
