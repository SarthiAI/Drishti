//! PII check. Two stages: the always-on regex set (fast, structural) and an
//! optional model-backed NER stage (unstructured PII in prose). Spans from both
//! are merged, overlaps resolved, and redaction applied per the operator policy.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Instant;

use drishti_regex::{scan, REGEX_VERSION};
use sha2::{Digest, Sha256};

use crate::config::{NerConfig, PiiConfig, RedactionStrategy};
use crate::engine::{softmax, InferenceModel};
use crate::error::DrishtiError;
use crate::types::{PiiCheck, PiiKind, PiiSource, PiiSpan, Validation};

/// Process-global tokenisation registry: maps a seen value to a stable token,
/// so the same value redacts to the same token across calls within a process.
/// Not persisted across restarts, per the v0.1 contract.
static TOKEN_REGISTRY: LazyLock<Mutex<(HashMap<String, String>, u64)>> =
    LazyLock::new(|| Mutex::new((HashMap::new(), 0)));

pub fn run(
    cfg: &PiiConfig,
    ner_model: Option<&InferenceModel>,
    input: &str,
) -> Result<PiiCheck, DrishtiError> {
    let start = Instant::now();
    let mut spans: Vec<PiiSpan> = Vec::new();

    if cfg.regex_enabled {
        for m in scan(input) {
            spans.push(PiiSpan {
                start: m.start,
                end: m.end,
                kind: PiiKind::from_tag(m.kind),
                confidence: m.confidence,
                source: PiiSource::Regex,
            });
        }
    }

    let mut ner_model_id = None;
    if let (Some(ner_cfg), Some(model)) = (cfg.ner.as_ref(), ner_model) {
        ner_model_id = Some(model.model_id.clone());
        spans.extend(run_ner(model, ner_cfg, input)?);
    }

    let kept = resolve_overlaps(spans);
    let (redacted, refuse) = redact(input, &kept, cfg);

    Ok(PiiCheck {
        spans: kept,
        redacted,
        refuse,
        latency_ms: start.elapsed().as_millis() as u32,
        regex_version: REGEX_VERSION.to_string(),
        ner_model_id,
        validation: Validation::Experimental,
    })
}

/// Greedy overlap resolution: take spans left to right and drop any that overlap
/// one already kept. Among spans sharing a start, the higher-confidence (then
/// longer) one wins, so a specific kind like Aadhaar beats a greedy generic like
/// Phone on the same digits. A redaction must not double-cover bytes.
fn resolve_overlaps(mut spans: Vec<PiiSpan>) -> Vec<PiiSpan> {
    spans.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then(
                b.confidence
                    .partial_cmp(&a.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then(b.end.cmp(&a.end))
    });
    let mut kept: Vec<PiiSpan> = Vec::new();
    let mut cursor = 0usize;
    for s in spans {
        if s.start >= cursor && s.end > s.start {
            cursor = s.end;
            kept.push(s);
        }
    }
    kept
}

/// Build the redacted string. `kept` is non-overlapping and sorted by start.
/// Returns the redacted text and whether any span requested refusal.
fn redact(input: &str, kept: &[PiiSpan], cfg: &PiiConfig) -> (String, bool) {
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0usize;
    let mut refuse = false;

    for span in kept {
        out.push_str(&input[cursor..span.start]);
        let value = &input[span.start..span.end];
        let strategy = cfg.redaction.strategy_for(&span.kind.label());
        match strategy {
            RedactionStrategy::Keep => out.push_str(value),
            RedactionStrategy::Mask => out.push_str(&mask_marker(&span.kind)),
            RedactionStrategy::Hash => out.push_str(&hash_marker(&span.kind, value)),
            RedactionStrategy::Tokenise => out.push_str(&tokenise(&span.kind, value)),
            RedactionStrategy::Refuse => {
                refuse = true;
                out.push_str(&mask_marker(&span.kind));
            }
        }
        cursor = span.end;
    }
    out.push_str(&input[cursor..]);
    (out, refuse)
}

fn mask_marker(kind: &PiiKind) -> String {
    format!("[{}]", kind.label().to_uppercase())
}

fn hash_marker(kind: &PiiKind, value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    let hex = hex::encode(hasher.finalize());
    format!("[{}#{}]", kind.label().to_uppercase(), &hex[..12])
}

fn tokenise(kind: &PiiKind, value: &str) -> String {
    let mut guard = TOKEN_REGISTRY.lock().expect("token registry poisoned");
    if let Some(existing) = guard.0.get(value) {
        return existing.clone();
    }
    guard.1 += 1;
    let token = format!("[{}_{}]", kind.label().to_uppercase(), guard.1);
    guard.0.insert(value.to_string(), token.clone());
    token
}

/// NER stage: per-token argmax over the configured BIO labels, merged into
/// entity spans using the tokenizer's byte offsets.
fn run_ner(
    model: &InferenceModel,
    cfg: &NerConfig,
    input: &str,
) -> Result<Vec<PiiSpan>, DrishtiError> {
    let (rows, offsets, _truncated) = model.classify_tokens(input)?;
    let mut spans = Vec::new();

    // Accumulator for the entity currently being built.
    let mut cur: Option<(String, usize, usize, f32, usize)> = None; // type, start, end, conf_sum, count

    let flush = |cur: &mut Option<(String, usize, usize, f32, usize)>, spans: &mut Vec<PiiSpan>| {
        if let Some((ty, start, end, conf_sum, count)) = cur.take() {
            spans.push(PiiSpan {
                start,
                end,
                kind: PiiKind::from_tag(&ty),
                confidence: conf_sum / count.max(1) as f32,
                source: PiiSource::Ner,
            });
        }
    };

    for (row, &(off_start, off_end)) in rows.iter().zip(offsets.iter()) {
        // Special tokens carry a zero-width offset; skip them.
        if off_end <= off_start {
            continue;
        }
        let probs = softmax(row);
        let (best_idx, &best_p) = probs
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap_or((0, &0.0));

        let label = cfg.labels.get(best_idx).map(String::as_str).unwrap_or("O");
        let (prefix, ty) = parse_bio(label);

        let below = best_p < cfg.threshold;
        if prefix == 'O' || ty.is_empty() || below {
            flush(&mut cur, &mut spans);
            continue;
        }

        match &mut cur {
            Some((cur_ty, _start, end, conf_sum, count)) if prefix == 'I' && cur_ty == ty => {
                *end = off_end;
                *conf_sum += best_p;
                *count += 1;
            }
            _ => {
                flush(&mut cur, &mut spans);
                cur = Some((ty.to_string(), off_start, off_end, best_p, 1));
            }
        }
    }
    flush(&mut cur, &mut spans);
    // Subword tokens and adjacent same-type entities ("Rahul" + "Sharma") arrive
    // as separate spans. Merge same-kind spans that are contiguous or separated
    // only by whitespace into one entity.
    let mut spans = merge_adjacent(input, spans);
    // Optional precision filter: drop short all-caps acronyms that generic NER
    // models mislabel as organisations (PAN, VAT, IBAN, ...).
    if cfg.drop_acronyms {
        spans.retain(|s| !is_acronym(&input[s.start..s.end]));
    }
    Ok(spans)
}

/// True for a short, all-uppercase ASCII token (2 to 6 letters).
fn is_acronym(text: &str) -> bool {
    let len = text.chars().count();
    (2..=6).contains(&len) && !text.is_empty() && text.chars().all(|c| c.is_ascii_uppercase())
}

/// Merge same-kind spans separated only by whitespace (or directly adjacent).
fn merge_adjacent(text: &str, mut spans: Vec<PiiSpan>) -> Vec<PiiSpan> {
    spans.sort_by_key(|s| s.start);
    let mut out: Vec<PiiSpan> = Vec::new();
    for s in spans {
        if let Some(last) = out.last_mut() {
            if last.kind == s.kind && s.start >= last.end {
                let gap = &text[last.end..s.start];
                if gap.chars().all(char::is_whitespace) {
                    last.end = s.end.max(last.end);
                    last.confidence = last.confidence.min(s.confidence);
                    continue;
                }
            }
        }
        out.push(s);
    }
    out
}

/// Split a BIO label into its prefix and entity type. `O` and unknown shapes
/// return prefix `O` with an empty type.
fn parse_bio(label: &str) -> (char, &str) {
    if label == "O" || label.is_empty() {
        return ('O', "");
    }
    match label.split_once('-') {
        Some((p, ty)) => (p.chars().next().unwrap_or('O'), ty),
        None => ('B', label),
    }
}
