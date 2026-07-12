//! The `ModelSource` trait. Defined in core (not in `drishti-models`) so the
//! abstraction lives with the consumer and core takes on no HTTP-client
//! dependency. The concrete implementations live in `drishti-models`. Keeping the
//! trait here is what lets the core stay pure, with no HTTP-client dependency.

use std::path::PathBuf;

use crate::config::Artifact;
use crate::error::ModelError;

/// Turns a configured [`Artifact`] into a concrete path on disk.
///
/// Implementations follow present-or-fetch (ADR-004): if the artifact is already
/// available locally, return it directly (verifying its hash when one is given);
/// if it is absent and remote, download it first, verify, cache, then return; if
/// it is absent with no remote source, fail loudly.
///
/// `fetch` is synchronous on purpose: model resolution is one-time setup, and a
/// blocking call keeps an async runtime out of the loader path.
pub trait ModelSource: Send + Sync {
    fn fetch(&self, id: &str, artifact: &Artifact) -> Result<PathBuf, ModelError>;
}
