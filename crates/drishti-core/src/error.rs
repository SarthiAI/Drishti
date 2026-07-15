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

/// The error a [`crate::SafetyEngine`] returns. It exists to separate two things
/// a host must treat differently:
///
/// - [`SafetyError::Engine`] is a real result from an available engine that
///   failed to compute (bad model, bad input, misconfiguration). It is a fault,
///   not a verdict, but the engine was reachable.
/// - [`SafetyError::Unavailable`] means the safety backend could not be reached
///   or did not answer: a transport failure, a timeout, a non-2xx response, or a
///   model-set mismatch when the check runs across a network. A host MUST fail
///   **closed** on this (block the traffic), never silently allow. This is the
///   whole reason the trait carries its own error type instead of reusing
///   [`DrishtiError`]: an embedded engine is always present, but a remote one is
///   not, and the two cases must be distinguishable at the call site.
#[derive(Debug, thiserror::Error)]
pub enum SafetyError {
    /// The engine was reachable but failed to produce a result.
    #[error("engine error: {0}")]
    Engine(#[from] DrishtiError),

    /// The safety backend is unreachable, timed out, answered non-2xx, or is
    /// serving a different model set than the caller pinned. Fail closed.
    #[error("safety backend unavailable: {0}")]
    Unavailable(String),
}
