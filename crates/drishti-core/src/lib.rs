//! Drishti core: three content-safety checks over text passing through LLM
//! systems. Prompt injection and PII on inputs, output safety on outputs. Every
//! check returns a calibrated score and the identity of the model that produced
//! it. No check makes a policy decision.
//!
//! This crate is pure of `tokio`, HTTP clients, and `pyo3`. Model bytes reach it
//! through the [`ModelSource`] trait, which the caller supplies. The concrete
//! loaders live in `drishti-models`.

mod checks;
pub mod config;
mod engine;
pub mod error;
mod source;
pub mod types;

use config::{DrishtiConfig, ModelEntry, OutputConfig, PiiConfig, PromptConfig};
use engine::InferenceModel;
use error::DrishtiError;

pub use source::ModelSource;
pub use types::{
    FullCheck, ModelManifest, ModelManifestEntry, OutputCheck, PiiCheck, PiiKind, PiiSource,
    PiiSpan, PromptCheck, PromptClass, SafetyVerdict, Validation,
};

struct PromptEngine {
    model: InferenceModel,
    cfg: PromptConfig,
}

struct PiiEngine {
    ner: Option<InferenceModel>,
    cfg: PiiConfig,
}

struct OutputEngine {
    model: InferenceModel,
    cfg: OutputConfig,
}

/// The built, immutable Drishti instance. Construct it through [`Drishti::builder`].
pub struct Drishti {
    prompt: Option<PromptEngine>,
    pii: Option<PiiEngine>,
    output: Option<OutputEngine>,
}

impl Drishti {
    pub fn builder() -> DrishtiBuilder {
        DrishtiBuilder {
            config: DrishtiConfig::default(),
        }
    }

    /// Prompt-injection check on an input string.
    pub async fn check_prompt(&self, input: &str) -> Result<PromptCheck, DrishtiError> {
        let e = self
            .prompt
            .as_ref()
            .ok_or(DrishtiError::CheckNotEnabled("prompt"))?;
        checks::prompt::run(&e.model, &e.cfg, input)
    }

    /// PII detection and redaction on an input string.
    pub async fn check_pii(&self, input: &str) -> Result<PiiCheck, DrishtiError> {
        let e = self.pii.as_ref().ok_or(DrishtiError::CheckNotEnabled("pii"))?;
        checks::pii::run(&e.cfg, e.ner.as_ref(), input)
    }

    /// Output-safety check on a model output string.
    pub async fn check_output(&self, output: &str) -> Result<OutputCheck, DrishtiError> {
        let e = self
            .output
            .as_ref()
            .ok_or(DrishtiError::CheckNotEnabled("output"))?;
        checks::output::run(&e.model, &e.cfg, output)
    }

    /// Run every enabled check. Checks that are not configured are simply
    /// absent from the result. (v0.1 runs them sequentially; a parallel driver
    /// over a session pool is recorded as backlog.)
    pub async fn check_all(
        &self,
        prompt: &str,
        output: Option<&str>,
    ) -> Result<FullCheck, DrishtiError> {
        let mut full = FullCheck::default();
        if self.prompt.is_some() {
            full.prompt = Some(self.check_prompt(prompt).await?);
        }
        if self.pii.is_some() {
            full.pii = Some(self.check_pii(prompt).await?);
        }
        if let (Some(_), Some(text)) = (self.output.as_ref(), output) {
            full.output = Some(self.check_output(text).await?);
        }
        Ok(full)
    }

    /// The loaded model identities, for audit. See invariant I6.
    pub fn model_manifest(&self) -> ModelManifest {
        let mut models = Vec::new();
        let mut push = |role: &str, m: &InferenceModel| {
            models.push(ModelManifestEntry {
                role: role.to_string(),
                model_id: m.model_id.clone(),
                sha256: m.sha256.clone(),
            });
        };
        if let Some(e) = &self.prompt {
            push("prompt", &e.model);
        }
        if let Some(e) = &self.pii {
            if let Some(m) = &e.ner {
                push("pii-ner", m);
            }
        }
        if let Some(e) = &self.output {
            push("output", &e.model);
        }
        ModelManifest {
            regex_version: drishti_regex::REGEX_VERSION.to_string(),
            models,
        }
    }
}

/// Builds a [`Drishti`] from configuration, resolving and loading every enabled
/// check's model through the supplied [`ModelSource`].
pub struct DrishtiBuilder {
    config: DrishtiConfig,
}

impl DrishtiBuilder {
    pub fn from_config(config: DrishtiConfig) -> Self {
        Self { config }
    }

    pub fn with_config(mut self, config: DrishtiConfig) -> Self {
        self.config = config;
        self
    }

    /// Resolve and load all enabled checks. Model resolution follows
    /// present-or-fetch via the source (ADR-004). Fails loudly if an enabled
    /// check has no usable model.
    pub fn build(self, source: &dyn ModelSource) -> Result<Drishti, DrishtiError> {
        let cfg = self.config;
        let intra = cfg.intra_threads;

        let prompt = match cfg.prompt {
            Some(pc) => {
                let model = load_model(source, &pc.model, pc.max_tokens, intra)?;
                Some(PromptEngine { model, cfg: pc })
            }
            None => None,
        };

        let pii = match cfg.pii {
            Some(pii_cfg) => {
                let ner = match pii_cfg.ner.as_ref() {
                    Some(ner_cfg) => {
                        Some(load_model(source, &ner_cfg.model, ner_cfg.max_tokens, intra)?)
                    }
                    None => None,
                };
                Some(PiiEngine { ner, cfg: pii_cfg })
            }
            None => None,
        };

        let output = match cfg.output {
            Some(oc) => {
                let model = load_model(source, &oc.model, oc.max_tokens, intra)?;
                Some(OutputEngine { model, cfg: oc })
            }
            None => None,
        };

        if prompt.is_none() && pii.is_none() && output.is_none() {
            return Err(DrishtiError::InvalidConfiguration(
                "no checks enabled: configure at least one of [prompt], [pii], [output]".into(),
            ));
        }

        Ok(Drishti {
            prompt,
            pii,
            output,
        })
    }
}

/// Resolve a model's two artifacts to paths via the source, then load them.
fn load_model(
    source: &dyn ModelSource,
    entry: &ModelEntry,
    max_tokens: usize,
    intra: Option<usize>,
) -> Result<InferenceModel, DrishtiError> {
    let model_path = source.fetch(&entry.id, &entry.model)?;
    let tokenizer_path = source.fetch(&entry.id, &entry.tokenizer)?;
    InferenceModel::load(&model_path, &tokenizer_path, entry.id.clone(), max_tokens, intra)
}
