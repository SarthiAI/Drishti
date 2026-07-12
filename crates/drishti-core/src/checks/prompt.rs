//! Prompt-injection check. Sequence classification over the input, mapped to an
//! injection score in [0, 1]. The positive-label index is configurable because
//! the label order belongs to the chosen model, not to Drishti.

use std::time::Instant;

use crate::config::PromptConfig;
use crate::engine::{sigmoid, softmax, InferenceModel};
use crate::error::DrishtiError;
use crate::types::{PromptCheck, PromptClass, Validation};

pub fn run(model: &InferenceModel, cfg: &PromptConfig, input: &str) -> Result<PromptCheck, DrishtiError> {
    let start = Instant::now();
    let (logits, truncated) = model.classify_sequence(input)?;

    let (score, confidence) = if logits.len() >= 2 {
        let probs = softmax(&logits);
        let idx = cfg.positive_label.min(probs.len() - 1);
        let top = probs.iter().cloned().fold(0.0_f32, f32::max);
        (probs[idx], top)
    } else if logits.len() == 1 {
        let s = sigmoid(logits[0]);
        (s, (s - 0.5).abs() * 2.0)
    } else {
        return Err(DrishtiError::InferenceFailed(
            "prompt-injection model returned no logits".into(),
        ));
    };

    // v0.1 emits the binary outcome. Finer sub-classes stay experimental and are
    // not assigned here until labelled data validates them.
    let class = if score >= 0.5 {
        PromptClass::InstructionOverride
    } else {
        PromptClass::Benign
    };

    Ok(PromptCheck {
        score,
        class,
        confidence,
        latency_ms: start.elapsed().as_millis() as u32,
        model_id: model.model_id.clone(),
        truncated,
        // Stays experimental until the eval harness (P05) clears the bar.
        validation: Validation::Experimental,
    })
}
