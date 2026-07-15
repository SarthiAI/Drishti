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

use ort::session::builder::{GraphOptimizationLevel, SessionBuilder};
use ort::session::{Session, SessionInputValue};
use ort::value::Tensor;
use sha2::{Digest, Sha256};
use tokenizers::{Tokenizer, TruncationParams};

use crate::config::ExecutionProvider;
use crate::error::DrishtiError;

/// How a session is built: execution provider, GPU device, and intra-op threads.
/// Assembled from [`crate::config::DrishtiConfig`] and passed to every model
/// load, so all of a Drishti instance's sessions share one runtime setup.
#[derive(Clone, Copy, Debug)]
pub struct SessionOptions {
    pub provider: ExecutionProvider,
    pub device_id: i32,
    pub intra_threads: Option<usize>,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            provider: ExecutionProvider::Cpu,
            device_id: 0,
            intra_threads: None,
        }
    }
}

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
        opts: &SessionOptions,
    ) -> Result<Self, DrishtiError> {
        let sha256 = sha256_file(model_path)?;

        let mut builder = Session::builder()
            .map_err(|e| DrishtiError::InferenceFailed(format!("session builder: {e}")))?;
        builder = builder
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| DrishtiError::InferenceFailed(format!("optimization level: {e}")))?;
        // Register the requested execution provider (CPU by default). This is the
        // one place the GPU option touches session setup; on a CPU build or a CPU
        // choice it is a no-op and the session is byte-for-byte today's.
        builder = apply_execution_provider(builder, opts)?;
        if let Some(threads) = opts.intra_threads {
            builder = builder
                .with_intra_threads(threads)
                .map_err(|e| DrishtiError::InferenceFailed(format!("intra threads: {e}")))?;
        }
        let session = builder
            .commit_from_file(model_path)
            .map_err(|e| DrishtiError::InferenceFailed(format!("load {model_id}: {e}")))?;

        let input_names = session.inputs().iter().map(|i| i.name().to_string()).collect();

        let mut tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|e| DrishtiError::TokenizationFailed(format!("load tokenizer: {e}")))?;

        // Make the engine's `max_tokens` the authoritative truncation length,
        // regardless of what the tokenizer file specified. The tokenizer does
        // special-token-aware truncation (it keeps the model's [CLS]/[SEP]
        // framing), and when it actually drops tokens the overflow is recorded,
        // which is how `encode` reports a real `truncated` flag.
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: max_tokens,
                ..Default::default()
            }))
            .map_err(|e| DrishtiError::TokenizationFailed(format!("set truncation: {e}")))?;

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

        // The tokenizer truncates to `max_tokens` (set at load); when it drops
        // tokens the removed remainder is recorded as overflow, so a non-empty
        // overflow means the input was genuinely truncated.
        let truncated = !enc.get_overflowing().is_empty();

        let ids = enc.get_ids();
        let mask = enc.get_attention_mask();
        let offsets = enc.get_offsets();

        // Defensive cap: the tokenizer already bounds length, but never feed the
        // model more than `max_tokens` even if a tokenizer file disables it.
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

    /// Batched sequence classification. Encodes each input, pads every sequence
    /// to the batch's longest, runs one forward pass, and returns one
    /// `(logits, truncated)` pair per input, in input order. Padding positions
    /// carry attention mask 0, so a padded row's logits equal the row's own
    /// single-input logits: a batched verdict is identical to a single one. This
    /// is what makes server-side batching safe (invariant: batching never
    /// changes a decision).
    pub fn classify_sequence_batch(
        &self,
        texts: &[&str],
    ) -> Result<Vec<(Vec<f32>, bool)>, DrishtiError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let encs: Vec<Encoded> = texts
            .iter()
            .map(|t| self.encode(t))
            .collect::<Result<_, _>>()?;
        let maxlen = encs.iter().map(|e| e.ids.len()).max().unwrap_or(0);
        if maxlen == 0 {
            return Err(DrishtiError::InferenceFailed(
                "batch produced an empty token encoding".into(),
            ));
        }
        let feeds = self.feeds_batch(&encs, maxlen)?;
        let flat = self.run_raw(feeds)?;
        let n = texts.len();
        if flat.len() % n != 0 {
            return Err(DrishtiError::InferenceFailed(format!(
                "batch logits length {} not divisible by batch size {n}",
                flat.len()
            )));
        }
        let num_labels = flat.len() / n;
        let out = (0..n)
            .map(|i| {
                (
                    flat[i * num_labels..(i + 1) * num_labels].to_vec(),
                    encs[i].truncated,
                )
            })
            .collect();
        Ok(out)
    }

    /// Build the padded `[batch, maxlen]` feeds for a batch of encodings. Rows
    /// shorter than `maxlen` are right-padded with token id 0 and attention mask
    /// 0 so the model ignores them.
    fn feeds_batch(
        &self,
        encs: &[Encoded],
        maxlen: usize,
    ) -> Result<Vec<(String, SessionInputValue<'static>)>, DrishtiError> {
        let n = encs.len();
        let shape = vec![n as i64, maxlen as i64];
        let mut ids = Vec::with_capacity(n * maxlen);
        let mut mask = Vec::with_capacity(n * maxlen);
        let mut typ = Vec::with_capacity(n * maxlen);
        for e in encs {
            for j in 0..maxlen {
                if j < e.ids.len() {
                    ids.push(e.ids[j]);
                    mask.push(e.mask[j]);
                } else {
                    ids.push(0);
                    mask.push(0);
                }
                typ.push(0_i64);
            }
        }
        let mut feeds: Vec<(String, SessionInputValue<'static>)> = Vec::new();
        for name in &self.input_names {
            let tensor = match name.as_str() {
                "input_ids" => Tensor::from_array((shape.clone(), ids.clone())),
                "attention_mask" => Tensor::from_array((shape.clone(), mask.clone())),
                "token_type_ids" => Tensor::from_array((shape.clone(), typ.clone())),
                _ => continue,
            }
            .map_err(|e| DrishtiError::InferenceFailed(format!("batch tensor {name}: {e}")))?;
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
}

/// Register the configured execution provider on a session builder. CPU is a
/// no-op (the ORT default). The GPU providers only exist in a build compiled
/// with the matching cargo feature; on a CPU-only build an explicit GPU choice
/// returns a clear error so a misconfigured node fails closed rather than
/// silently running on the CPU.
fn apply_execution_provider(
    builder: SessionBuilder,
    opts: &SessionOptions,
) -> Result<SessionBuilder, DrishtiError> {
    match opts.provider {
        ExecutionProvider::Cpu => Ok(builder),
        ExecutionProvider::Cuda => {
            eprintln!(
                "drishti: execution_provider=cuda (device {})",
                opts.device_id
            );
            add_cuda(builder, opts.device_id, true)
        }
        ExecutionProvider::Tensorrt => {
            eprintln!(
                "drishti: execution_provider=tensorrt (device {})",
                opts.device_id
            );
            add_tensorrt(builder, opts.device_id, true)
        }
        ExecutionProvider::Auto => auto_provider(builder, opts.device_id),
    }
}

/// `auto`: try a GPU provider if one is compiled in, otherwise use the CPU.
/// Never errors on account of a missing GPU (soft fallback), and logs the
/// choice so an operator can confirm what actually ran.
fn auto_provider(builder: SessionBuilder, device_id: i32) -> Result<SessionBuilder, DrishtiError> {
    #[cfg(feature = "cuda")]
    {
        eprintln!("drishti: execution_provider=auto, registering CUDA with CPU fallback");
        return add_cuda(builder, device_id, false);
    }
    #[cfg(all(not(feature = "cuda"), feature = "tensorrt"))]
    {
        eprintln!("drishti: execution_provider=auto, registering TensorRT with CPU fallback");
        return add_tensorrt(builder, device_id, false);
    }
    #[cfg(all(not(feature = "cuda"), not(feature = "tensorrt")))]
    {
        let _ = device_id;
        eprintln!("drishti: execution_provider=auto but no GPU provider is compiled in; using CPU");
        Ok(builder)
    }
}

#[cfg(feature = "cuda")]
fn add_cuda(
    builder: SessionBuilder,
    device_id: i32,
    hard: bool,
) -> Result<SessionBuilder, DrishtiError> {
    use ort::execution_providers::CUDAExecutionProvider;
    let mut dispatch = CUDAExecutionProvider::default()
        .with_device_id(device_id)
        .build();
    if hard {
        dispatch = dispatch.error_on_failure();
    }
    builder
        .with_execution_providers([dispatch])
        .map_err(|e| DrishtiError::InferenceFailed(format!("register CUDA execution provider: {e}")))
}

#[cfg(not(feature = "cuda"))]
fn add_cuda(
    _builder: SessionBuilder,
    _device_id: i32,
    _hard: bool,
) -> Result<SessionBuilder, DrishtiError> {
    Err(DrishtiError::InvalidConfiguration(
        "execution_provider = \"cuda\" requires a build with --features cuda; this binary is \
         CPU-only. Rebuild drishti-core/drishti-server with the cuda feature, or use \
         execution_provider = \"auto\" to fall back to the CPU."
            .into(),
    ))
}

#[cfg(feature = "tensorrt")]
fn add_tensorrt(
    builder: SessionBuilder,
    device_id: i32,
    hard: bool,
) -> Result<SessionBuilder, DrishtiError> {
    use ort::execution_providers::TensorRTExecutionProvider;
    let mut dispatch = TensorRTExecutionProvider::default()
        .with_device_id(device_id)
        .build();
    if hard {
        dispatch = dispatch.error_on_failure();
    }
    builder.with_execution_providers([dispatch]).map_err(|e| {
        DrishtiError::InferenceFailed(format!("register TensorRT execution provider: {e}"))
    })
}

#[cfg(not(feature = "tensorrt"))]
fn add_tensorrt(
    _builder: SessionBuilder,
    _device_id: i32,
    _hard: bool,
) -> Result<SessionBuilder, DrishtiError> {
    Err(DrishtiError::InvalidConfiguration(
        "execution_provider = \"tensorrt\" requires a build with --features tensorrt; this binary \
         is CPU-only. Rebuild with the tensorrt feature, or use execution_provider = \"auto\" to \
         fall back to the CPU."
            .into(),
    ))
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
