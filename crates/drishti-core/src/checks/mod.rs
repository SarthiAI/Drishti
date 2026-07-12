//! The three checks. Each is a free function over a loaded model (or, for PII,
//! the regex set plus an optional model) and its config. They hold no state and
//! make no policy decision: they compute a score and return it.

pub mod output;
pub mod pii;
pub mod prompt;
