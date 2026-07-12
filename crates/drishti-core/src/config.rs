//! Configuration. This is where every model choice lives. Nothing about model
//! identity is hardcoded anywhere in the binary: which model each check uses,
//! where it comes from, and its optional integrity hash all arrive here, at
//! runtime, from the operator. See ADR-003 and invariant I2.

use std::collections::HashMap;
use std::path::PathBuf;

use figment::providers::{Env, Format, Toml};
use figment::Figment;
use serde::Deserialize;

use crate::error::DrishtiError;

/// Where an artifact's bytes come from.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    /// A path on the local filesystem. Used directly, never downloaded.
    Local,
    /// A URL to download from if the file is not already cached.
    Remote,
}

/// One downloadable or local file: a model graph, or a tokenizer. The unit the
/// `ModelSource` resolves to a concrete path on disk.
#[derive(Clone, Debug, Deserialize)]
pub struct Artifact {
    pub source: SourceKind,
    /// A filesystem path (local) or a URL (remote).
    pub location: String,
    /// Optional SHA-256. When present it is enforced strictly; when absent the
    /// artifact is used as-is. Model identity is operator-chosen, so hashes are
    /// opt-in per artifact rather than pinned in the binary.
    #[serde(default)]
    pub sha256: Option<String>,
}

/// A model: its logical id (cache key and audit identity), the ONNX graph, and
/// the tokenizer that graph was trained with.
#[derive(Clone, Debug, Deserialize)]
pub struct ModelEntry {
    pub id: String,
    pub model: Artifact,
    pub tokenizer: Artifact,
}

fn default_max_tokens() -> usize {
    512
}
fn default_positive_label() -> usize {
    1
}
fn default_output_threshold() -> f32 {
    0.7
}
fn default_ner_threshold() -> f32 {
    0.5
}
fn default_true() -> bool {
    true
}

/// Prompt-injection check configuration.
#[derive(Clone, Debug, Deserialize)]
pub struct PromptConfig {
    pub model: ModelEntry,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
    /// Index of the "injection" logit in the model's output. Default 1 (the
    /// common benign=0 / injection=1 layout). Configurable because the label
    /// order is a property of the chosen model, not of Drishti.
    #[serde(default = "default_positive_label")]
    pub positive_label: usize,
}

/// Redaction strategy applied to a span, chosen per kind by the operator.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum RedactionStrategy {
    /// Replace with a fixed marker, e.g. `[EMAIL]`.
    #[default]
    Mask,
    /// Replace with a truncated SHA-256 hex of the value (deterministic).
    Hash,
    /// Replace with a stable per-process random token.
    Tokenise,
    /// Leave the value in place (detect-and-log only).
    Keep,
    /// Mark the span and let the caller refuse the whole request.
    Refuse,
}

/// How spans get redacted. A default strategy plus per-kind overrides.
#[derive(Clone, Debug, Deserialize)]
pub struct RedactionPolicy {
    #[serde(default)]
    pub default: RedactionStrategy,
    /// Override by kind name (e.g. "Email", "CreditCard").
    #[serde(default)]
    pub per_kind: HashMap<String, RedactionStrategy>,
}

impl Default for RedactionPolicy {
    fn default() -> Self {
        Self {
            default: RedactionStrategy::Mask,
            per_kind: HashMap::new(),
        }
    }
}

impl RedactionPolicy {
    pub fn strategy_for(&self, kind: &str) -> RedactionStrategy {
        self.per_kind.get(kind).copied().unwrap_or(self.default)
    }
}

/// Optional model-backed NER stage for unstructured PII.
#[derive(Clone, Debug, Deserialize)]
pub struct NerConfig {
    pub model: ModelEntry,
    /// Per-logit labels in BIO scheme, aligned to the model's output order,
    /// e.g. `["O", "B-PER", "I-PER", "B-LOC", "I-LOC"]`. The model defines the
    /// set; the config states it, so nothing is hardcoded.
    pub labels: Vec<String>,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
    #[serde(default = "default_ner_threshold")]
    pub threshold: f32,
    /// Drop NER spans that look like short all-caps acronyms (2 to 6 letters,
    /// e.g. "PAN", "VAT"). General-purpose NER models tag these as
    /// organisations; enabling this trades a little org recall for precision.
    #[serde(default)]
    pub drop_acronyms: bool,
}

/// PII check configuration.
#[derive(Clone, Debug, Deserialize)]
pub struct PiiConfig {
    /// The always-on, fast regex stage.
    #[serde(default = "default_true")]
    pub regex_enabled: bool,
    /// The optional model-backed stage.
    #[serde(default)]
    pub ner: Option<NerConfig>,
    #[serde(default)]
    pub redaction: RedactionPolicy,
}

/// Output-safety check configuration.
#[derive(Clone, Debug, Deserialize)]
pub struct OutputConfig {
    pub model: ModelEntry,
    /// Category names aligned to the model's logit order. The taxonomy is a
    /// property of the chosen model and is stated here, never hardcoded.
    pub categories: Vec<String>,
    /// How to turn logits into per-category scores. `false` (default) means a
    /// single-label softmax model (one winning class, like a model with an
    /// explicit "safe/OK" class). `true` means a multi-label model where each
    /// category is an independent sigmoid.
    #[serde(default)]
    pub multi_label: bool,
    /// The category name that means "safe" (for example "OK"). When set, that
    /// category never triggers a failure and is excluded from the verdict. This
    /// is how a softmax model with a safe class is handled, with nothing about
    /// the taxonomy hardcoded.
    #[serde(default)]
    pub safe_category: Option<String>,
    #[serde(default = "default_output_threshold")]
    pub threshold: f32,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
}

/// The whole configuration. A check is enabled exactly when its section is
/// present. If a section is present its model must be fully specified, or the
/// build fails with a clear configuration error rather than guessing a default.
#[derive(Clone, Debug, Deserialize, Default)]
pub struct DrishtiConfig {
    /// Cache directory for downloaded models. Defaults to the platform cache
    /// dir under `drishti/models` when unset.
    #[serde(default)]
    pub cache_dir: Option<PathBuf>,
    /// Threads for ONNX intra-op parallelism. None lets the runtime decide.
    #[serde(default)]
    pub intra_threads: Option<usize>,
    #[serde(default)]
    pub prompt: Option<PromptConfig>,
    #[serde(default)]
    pub pii: Option<PiiConfig>,
    #[serde(default)]
    pub output: Option<OutputConfig>,
}

impl DrishtiConfig {
    /// Load configuration from a TOML string, then overlay environment-variable
    /// overrides. Any field is reachable as `DRISHTI_<PATH>` with `__` between
    /// nesting levels, for example `DRISHTI_OUTPUT__THRESHOLD=0.3` or
    /// `DRISHTI_PII__NER__DROP_ACRONYMS=true`. Callers that want `.env` file
    /// support should load it (e.g. via dotenvy) before calling this. No tunable
    /// requires a code change or a rebuild.
    pub fn from_toml_and_env(toml_text: &str) -> Result<Self, DrishtiError> {
        Figment::new()
            .merge(Toml::string(toml_text))
            .merge(Env::prefixed("DRISHTI_").split("__"))
            .extract()
            .map_err(|e| DrishtiError::InvalidConfiguration(e.to_string()))
    }
}
