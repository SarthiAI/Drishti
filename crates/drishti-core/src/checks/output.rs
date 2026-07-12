//! Output-safety check. Sequence classification over the model's output text,
//! producing per-category scores and an aggregate verdict against the configured
//! threshold. The taxonomy (category names and order) comes from config, never
//! hardcoded, so any classifier-style safety model can be plugged in.

use std::collections::HashMap;
use std::time::Instant;

use crate::config::OutputConfig;
use crate::engine::{sigmoid, softmax, InferenceModel};
use crate::error::DrishtiError;
use crate::types::{OutputCheck, SafetyVerdict, Validation};

pub fn run(model: &InferenceModel, cfg: &OutputConfig, output: &str) -> Result<OutputCheck, DrishtiError> {
    let start = Instant::now();
    let (logits, _truncated) = model.classify_sequence(output)?;

    if logits.len() != cfg.categories.len() || cfg.categories.is_empty() {
        return Err(DrishtiError::InvalidConfiguration(format!(
            "output model produced {} logits but the config lists {} categories",
            logits.len(),
            cfg.categories.len()
        )));
    }

    // Per-category scores. Multi-label models score each category independently
    // (sigmoid); single-label models score one winning class (softmax).
    let scores: Vec<f32> = if cfg.multi_label {
        logits.iter().map(|&l| sigmoid(l)).collect()
    } else {
        softmax(&logits)
    };

    let mut categories = HashMap::new();
    for (name, &score) in cfg.categories.iter().zip(scores.iter()) {
        categories.insert(name.clone(), score);
    }

    // Verdict. For a softmax model with a safe class, the unsafe probability is
    // spread across several classes, so the right signal is how little weight is
    // on the safe class: fail when (1 - P(safe)) reaches the threshold. For a
    // multi-label model each category is independent, so fail when any unsafe
    // category crosses the threshold. With no safe class, any category crossing
    // the threshold fails.
    let unsafe_triggered = match (cfg.safe_category.as_deref(), cfg.multi_label) {
        (Some(safe), false) => {
            let safe_score = categories.get(safe).copied().unwrap_or(0.0);
            (1.0 - safe_score) >= cfg.threshold
        }
        (Some(safe), true) => categories
            .iter()
            .any(|(name, &score)| name != safe && score >= cfg.threshold),
        (None, _) => categories.values().any(|&score| score >= cfg.threshold),
    };

    let overall = if unsafe_triggered {
        SafetyVerdict::Fail
    } else {
        SafetyVerdict::Pass
    };

    Ok(OutputCheck {
        categories,
        overall,
        language: detect_language(output),
        latency_ms: start.elapsed().as_millis() as u32,
        model_id: model.model_id.clone(),
        validation: Validation::Experimental,
    })
}

/// Lightweight language heuristic: if the letters are overwhelmingly ASCII we
/// report "en", otherwise "und" (undetermined). This is honest about being a
/// heuristic; a real language-id model is a configurable add-on for later.
fn detect_language(text: &str) -> String {
    let letters: Vec<char> = text.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.is_empty() {
        return "und".to_string();
    }
    let ascii = letters.iter().filter(|c| c.is_ascii_alphabetic()).count();
    if ascii as f32 / letters.len() as f32 >= 0.9 {
        "en".to_string()
    } else {
        "und".to_string()
    }
}
