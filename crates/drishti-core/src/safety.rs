//! The backend-agnostic safety seam. [`SafetyEngine`] captures exactly the three
//! checks a host runs around an LLM call, so a host can hold one
//! `Arc<dyn SafetyEngine>` and not care whether inference happens in-process or
//! across a network. The embedded [`Drishti`] implements it here; the remote
//! client (`drishti-client`) implements it over HTTP.
//!
//! The trait returns [`SafetyError`], not [`crate::error::DrishtiError`], so a
//! caller can tell a real verdict-time fault apart from an unreachable backend
//! and fail closed on the latter. See [`SafetyError`] for that split.

use async_trait::async_trait;

use crate::error::SafetyError;
use crate::types::{OutputCheck, PiiCheck, PromptCheck};
use crate::Drishti;

/// The three content-safety checks, abstracted over where they run. Every
/// implementation returns the same result shapes as the embedded engine; only
/// the location of the computation differs.
#[async_trait]
pub trait SafetyEngine: Send + Sync {
    /// Prompt-injection check on an input string.
    async fn check_prompt(&self, text: &str) -> Result<PromptCheck, SafetyError>;
    /// PII detection and redaction on an input string.
    async fn check_pii(&self, text: &str) -> Result<PiiCheck, SafetyError>;
    /// Output-safety check on a model output string.
    async fn check_output(&self, text: &str) -> Result<OutputCheck, SafetyError>;
}

/// The embedded engine is always present, so its only failure mode is a
/// verdict-time fault, mapped to [`SafetyError::Engine`]. It never returns
/// [`SafetyError::Unavailable`]. The inherent methods (which return
/// `DrishtiError`) stay the primary surface and are untouched; this impl just
/// re-expresses them behind the trait. The fully-qualified `Drishti::` calls
/// below select those inherent methods, not the trait, so there is no
/// recursion.
#[async_trait]
impl SafetyEngine for Drishti {
    async fn check_prompt(&self, text: &str) -> Result<PromptCheck, SafetyError> {
        Drishti::check_prompt(self, text)
            .await
            .map_err(SafetyError::from)
    }

    async fn check_pii(&self, text: &str) -> Result<PiiCheck, SafetyError> {
        Drishti::check_pii(self, text).await.map_err(SafetyError::from)
    }

    async fn check_output(&self, text: &str) -> Result<OutputCheck, SafetyError> {
        Drishti::check_output(self, text)
            .await
            .map_err(SafetyError::from)
    }
}
