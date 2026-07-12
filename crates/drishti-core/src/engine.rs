//! ONNX inference plus tokenization, wrapped so the checks do not touch `ort`
//! directly. One [`InferenceModel`] is one loaded model: its session, its
//! tokenizer, and its audit identity (id and hash).
//!
//! `ort`'s `Session::run` takes `&mut self`, so the session sits behind a mutex.
//! Inference is therefore serialized per model. A session pool is a future
//! optimization, recorded as backlog, not needed for correctness.

use std::io::Read;
use std::path::Path;
use std::sync::Mutex;

use ort::session::builder::GraphOptimizationLevel;
use ort::session::{Session, SessionInputValue};
use ort::value::Tensor;
use sha2::{Digest, Sha256};
use tokenizers::Tokenizer;

use crate::error::DrishtiError;

/// A loaded model: ONNX session, tokenizer, and audit identity.
pub struct InferenceModel {
    session: Mutex<Session>,
    tokenizer: Tokenizer,
    input_names: Vec<String>,
    max_tokens: usize,
    pub model_id: String,
    pub sha256: String,
}

/// A single encoded input: token ids, attention mask, byte offsets, and whether
/// the input was truncated to fit `max_tokens`.
struct Encoded {
    ids: Vec<i64>,
    mask: Vec<i64>,
    offsets: Vec<(usize, usize)>,
    truncated: bool,
}

impl InferenceModel {
    /// Load a model and its tokenizer from concrete on-disk paths. The id and
    /// hash are carried through to every result this model produces.
    pub fn load(
        model_path: &Path,
        tokenizer_path: &Path,
        model_id: String,
        max_tokens: usize,
        intra_threads: Option<usize>,
    ) -> Result<Self, DrishtiError> {
        let sha256 = sha256_file(model_path)?;

        let mut builder = Session::builder()
            .map_err(|e| DrishtiError::InferenceFailed(format!("session builder: {e}")))?;
        builder = builder
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| DrishtiError::InferenceFailed(format!("optimization level: {e}")))?;
        if let Some(threads) = intra_threads {
            builder = builder
                .with_intra_threads(threads)
                .map_err(|e| DrishtiError::InferenceFailed(format!("intra threads: {e}")))?;
        }
        let session = builder
            .commit_from_file(model_path)
            .map_err(|e| DrishtiError::InferenceFailed(format!("load {model_id}: {e}")))?;

        let input_names = session.inputs().iter().map(|i| i.name().to_string()).collect();

        let tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|e| DrishtiError::TokenizationFailed(format!("load tokenizer: {e}")))?;

        Ok(Self {
            session: Mutex::new(session),
            tokenizer,
            input_names,
            max_tokens,
            model_id,
            sha256,
        })
    }

    fn encode(&self, text: &str) -> Result<Encoded, DrishtiError> {
        let enc = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| DrishtiError::TokenizationFailed(e.to_string()))?;

        let ids = enc.get_ids();
        let mask = enc.get_attention_mask();
        let offsets = enc.get_offsets();

        let truncated = ids.len() > self.max_tokens;
        let take = ids.len().min(self.max_tokens);

        Ok(Encoded {
            ids: ids[..take].iter().map(|&v| v as i64).collect(),
            mask: mask[..take].iter().map(|&v| v as i64).collect(),
            offsets: offsets[..take].to_vec(),
            truncated,
        })
    }

    /// Build the named input feeds the loaded model actually declares. Standard
    /// transformer exports name their inputs `input_ids`, `attention_mask`, and
    /// (for BERT-family) `token_type_ids`. We provide exactly what the model
    /// asks for and ignore the rest, so one path serves DeBERTa, BERT, RoBERTa.
    fn feeds(&self, enc: &Encoded) -> Result<Vec<(String, SessionInputValue<'static>)>, DrishtiError> {
        let seq = enc.ids.len();
        let shape = vec![1_i64, seq as i64];
        let mut feeds: Vec<(String, SessionInputValue<'static>)> = Vec::new();
        for name in &self.input_names {
            let tensor = match name.as_str() {
                "input_ids" => Tensor::from_array((shape.clone(), enc.ids.clone())),
                "attention_mask" => Tensor::from_array((shape.clone(), enc.mask.clone())),
                "token_type_ids" => Tensor::from_array((shape.clone(), vec![0_i64; seq])),
                _ => continue,
            }
            .map_err(|e| DrishtiError::InferenceFailed(format!("tensor {name}: {e}")))?;
            feeds.push((name.clone(), tensor.into()));
        }
        if feeds.is_empty() {
            return Err(DrishtiError::InferenceFailed(format!(
                "model '{}' declares no recognized text inputs",
                self.model_id
            )));
        }
        Ok(feeds)
    }

    fn run_raw(&self, feeds: Vec<(String, SessionInputValue<'static>)>) -> Result<Vec<f32>, DrishtiError> {
        let mut session = self
            .session
            .lock()
            .map_err(|_| DrishtiError::InferenceFailed("session mutex poisoned".into()))?;
        let outputs = session
            .run(feeds)
            .map_err(|e| DrishtiError::InferenceFailed(format!("run {}: {e}", self.model_id)))?;
        let (_, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| DrishtiError::InferenceFailed(format!("extract logits: {e}")))?;
        Ok(data.to_vec())
    }

    /// Sequence classification: returns the flat logit vector (one per label,
    /// batch size one) and whether the input was truncated.
    pub fn classify_sequence(&self, text: &str) -> Result<(Vec<f32>, bool), DrishtiError> {
        let enc = self.encode(text)?;
        let feeds = self.feeds(&enc)?;
        let logits = self.run_raw(feeds)?;
        Ok((logits, enc.truncated))
    }

    /// Token classification: returns per-token logit rows, the byte offsets for
    /// each token, and the truncation flag. Used by the NER PII stage.
    pub fn classify_tokens(
        &self,
        text: &str,
    ) -> Result<(Vec<Vec<f32>>, Vec<(usize, usize)>, bool), DrishtiError> {
        let enc = self.encode(text)?;
        let seq = enc.ids.len();
        let offsets = enc.offsets.clone();
        let truncated = enc.truncated;
        let feeds = self.feeds(&enc)?;
        let flat = self.run_raw(feeds)?;

        if seq == 0 || flat.len() % seq != 0 {
            return Err(DrishtiError::InferenceFailed(format!(
                "token logits length {} not divisible by sequence length {seq}",
                flat.len()
            )));
        }
        let num_labels = flat.len() / seq;
        let rows = flat.chunks(num_labels).map(|r| r.to_vec()).collect();
        Ok((rows, offsets, truncated))
    }
}

/// Stream a file through SHA-256 without loading it all into memory.
fn sha256_file(path: &Path) -> Result<String, DrishtiError> {
    let mut file = std::fs::File::open(path).map_err(|e| DrishtiError::InferenceFailed(format!("open {}: {e}", path.display())))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| DrishtiError::InferenceFailed(format!("read {}: {e}", path.display())))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Numerically stable softmax over a logit slice.
pub fn softmax(logits: &[f32]) -> Vec<f32> {
    let max = logits.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let exps: Vec<f32> = logits.iter().map(|&l| (l - max).exp()).collect();
    let sum: f32 = exps.iter().sum();
    if sum == 0.0 {
        return vec![0.0; logits.len()];
    }
    exps.iter().map(|&e| e / sum).collect()
}

/// Logistic sigmoid, for multi-label outputs.
pub fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}
