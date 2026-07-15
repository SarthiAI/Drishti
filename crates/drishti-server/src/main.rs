//! `drishti-server`: the HTTP service. A thin adapter over `drishti-core`
//! (invariant I5): every endpoint deserializes a request, calls the same core
//! method the CLI calls, and serializes the result. No detection logic lives
//! here. Inference is blocking CPU (or GPU) work, so it runs on Tokio's blocking
//! pool to keep the async reactor free.
//!
//! Everything the P07 work order added is opt-in and lives in an optional
//! `[server]` config table (env `DRISHTI_SERVER__*`): TLS, a request-body limit,
//! a per-request timeout, a concurrency cap that sheds with 503, batch
//! endpoints, an optional operator-set model-set id, and a loud content-logging
//! debug flag. With none of it set the service behaves as it did before, except
//! that it now binds and answers `/healthz` and `/readyz` immediately and loads
//! models in the background, so `/readyz` is honestly false until the models are
//! up. Request and response CONTENT is never logged unless the debug flag is on.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use axum_server::tls_rustls::RustlsConfig;
use clap::Parser;
use drishti_core::config::DrishtiConfig;
use drishti_core::error::DrishtiError;
use drishti_core::{Drishti, FullCheck, ModelManifest, OutputCheck, PiiCheck, PromptCheck};
use drishti_models::FsSource;
use figment::providers::{Env, Format, Toml};
use figment::Figment;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const DRISHTI_VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_MAX_REQUEST_BYTES: usize = 1024 * 1024; // 1 MiB
const DEFAULT_REQUEST_TIMEOUT_MS: u64 = 30_000; // 30 s
const DEFAULT_MAX_BATCH: usize = 32;

#[derive(Parser)]
#[command(name = "drishti-server", about = "Drishti content-safety HTTP service")]
struct Args {
    /// TOML config selecting the models for each check (same schema as the CLI).
    /// An optional `[server]` table adds TLS, limits, and batching.
    #[arg(short, long)]
    config: PathBuf,
    /// Address to bind.
    #[arg(long, default_value = "0.0.0.0:8080")]
    bind: String,
    /// Static bearer token. Falls back to DRISHTI_TOKEN. If neither is set, the
    /// check endpoints run unauthenticated and a warning is logged.
    #[arg(long)]
    token: Option<String>,
}

/// Optional HTTP-service settings, read from the `[server]` table of the same
/// TOML file (and overridable via `DRISHTI_SERVER__*`). All fields are optional;
/// absent means today's behaviour.
#[derive(Debug, Default, Clone, Deserialize)]
struct ServerConfig {
    /// PEM certificate chain. TLS is on only when both cert and key are set.
    tls_cert_path: Option<PathBuf>,
    /// PEM private key.
    tls_key_path: Option<PathBuf>,
    /// Max request body in bytes (413 above this). Default 1 MiB.
    max_request_bytes: Option<usize>,
    /// Per-request timeout in ms (504 above this). Default 30000.
    request_timeout_ms: Option<u64>,
    /// Max in-flight check requests; extra requests get 503. Default unlimited.
    max_concurrency: Option<usize>,
    /// Max items accepted by a `/batch` endpoint (413 above this). Default 32.
    max_batch: Option<usize>,
    /// Operator-set model-set id. When unset it is derived from the loaded
    /// manifest. Clients pin this and get 409 on a mismatch.
    model_set: Option<String>,
    /// Log request/response CONTENT. Off by default, for local dev only. Loudly
    /// warned at startup when on, because content can carry end-user PII.
    #[serde(default)]
    log_content: bool,
}

impl ServerConfig {
    fn load(toml_text: &str) -> Self {
        let fig = Figment::new()
            .merge(Toml::string(toml_text))
            .merge(Env::prefixed("DRISHTI_").split("__"));
        // No [server] table (in the file or via env) is the normal case: use
        // defaults silently. A present-but-malformed one is surfaced loudly but
        // must not stop the service from starting.
        if fig.find_value("server").is_err() {
            return ServerConfig::default();
        }
        match fig.extract_inner::<ServerConfig>("server") {
            Ok(c) => c,
            Err(e) => {
                eprintln!("drishti-server: ignoring invalid [server] config: {e}");
                ServerConfig::default()
            }
        }
    }
}

#[derive(Clone)]
struct AppState(Arc<Inner>);

/// Set once the models finish loading. Its presence is what `/readyz` reports.
struct Loaded {
    engine: Drishti,
    model_set: String,
}

struct Inner {
    /// `None` until the background load completes.
    loaded: OnceLock<Loaded>,
    token: Option<String>,
    metrics: Metrics,
    /// `Some` when a concurrency cap is configured.
    semaphore: Option<Arc<Semaphore>>,
    request_timeout_ms: u64,
    max_batch: usize,
    log_content: bool,
}

#[derive(Default)]
struct Metrics {
    prompt: AtomicU64,
    pii: AtomicU64,
    output: AtomicU64,
    all: AtomicU64,
    errors: AtomicU64,
    busy: AtomicU64,
    latency_ms_sum: AtomicU64,
}

impl Metrics {
    fn render(&self, ready: bool, model_set: Option<&str>) -> String {
        let mut s = String::new();
        let mut counter = |name: &str, help: &str, labelled: &[(&str, u64)]| {
            s.push_str(&format!("# HELP {name} {help}\n# TYPE {name} counter\n"));
            for (label, value) in labelled {
                if label.is_empty() {
                    s.push_str(&format!("{name} {value}\n"));
                } else {
                    s.push_str(&format!("{name}{{endpoint=\"{label}\"}} {value}\n"));
                }
            }
        };
        counter(
            "drishti_requests_total",
            "Check requests handled, by endpoint.",
            &[
                ("prompt", self.prompt.load(Ordering::Relaxed)),
                ("pii", self.pii.load(Ordering::Relaxed)),
                ("output", self.output.load(Ordering::Relaxed)),
                ("all", self.all.load(Ordering::Relaxed)),
            ],
        );
        counter(
            "drishti_errors_total",
            "Check requests that returned an error.",
            &[("", self.errors.load(Ordering::Relaxed))],
        );
        counter(
            "drishti_busy_total",
            "Check requests shed with 503 because the concurrency limit was reached.",
            &[("", self.busy.load(Ordering::Relaxed))],
        );
        counter(
            "drishti_latency_ms_sum",
            "Cumulative reported check latency in milliseconds.",
            &[("", self.latency_ms_sum.load(Ordering::Relaxed))],
        );
        s.push_str("# HELP drishti_ready Whether models are loaded and the service is ready.\n");
        s.push_str("# TYPE drishti_ready gauge\n");
        s.push_str(&format!("drishti_ready {}\n", if ready { 1 } else { 0 }));
        if let Some(ms) = model_set {
            s.push_str("# HELP drishti_model_set The loaded model-set id.\n");
            s.push_str("# TYPE drishti_model_set gauge\n");
            s.push_str(&format!("drishti_model_set{{id=\"{ms}\"}} 1\n"));
        }
        s
    }
}

// Request bodies. Field names match the shipped contract exactly: `input` for
// prompt and pii, `output` for output. `model_set` is optional (serde default),
// so the existing SDKs, which send none, are unaffected; the 409 mismatch check
// fires only when a caller actually pins one.
#[derive(Deserialize)]
struct PromptReq {
    input: String,
    #[serde(default)]
    model_set: Option<String>,
}
#[derive(Deserialize)]
struct PiiReq {
    input: String,
    #[serde(default)]
    model_set: Option<String>,
}
#[derive(Deserialize)]
struct OutputReq {
    output: String,
    #[serde(default)]
    model_set: Option<String>,
}
#[derive(Deserialize)]
struct AllReq {
    prompt: String,
    #[serde(default)]
    output: Option<String>,
    #[serde(default)]
    model_set: Option<String>,
}
#[derive(Deserialize)]
struct PromptBatchReq {
    inputs: Vec<String>,
    #[serde(default)]
    model_set: Option<String>,
}
#[derive(Deserialize)]
struct PiiBatchReq {
    inputs: Vec<String>,
    #[serde(default)]
    model_set: Option<String>,
}
#[derive(Deserialize)]
struct OutputBatchReq {
    outputs: Vec<String>,
    #[serde(default)]
    model_set: Option<String>,
}

/// HTTP-shaped error. Maps core error variants to status codes.
struct ApiError {
    status: StatusCode,
    msg: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.msg }))).into_response()
    }
}

fn map_err(e: DrishtiError) -> ApiError {
    let status = match e {
        DrishtiError::CheckNotEnabled(_) => StatusCode::NOT_IMPLEMENTED,
        DrishtiError::InvalidConfiguration(_) | DrishtiError::InputTooLong { .. } => {
            StatusCode::BAD_REQUEST
        }
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    ApiError {
        status,
        msg: e.to_string(),
    }
}

fn record_err(st: &AppState, e: ApiError) -> ApiError {
    st.0.metrics.errors.fetch_add(1, Ordering::Relaxed);
    e
}

/// 503 until the background model load finishes. Returns the loaded state.
fn require_loaded(st: &AppState) -> Result<&Loaded, ApiError> {
    st.0.loaded.get().ok_or(ApiError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        msg: "models are still loading".into(),
    })
}

/// 409 when the caller pinned a model set that is not the one loaded. A caller
/// that pins nothing (the shipped SDKs) always passes.
fn check_model_set(requested: &Option<String>, server: &str) -> Result<(), ApiError> {
    if let Some(req) = requested {
        if req != server {
            return Err(ApiError {
                status: StatusCode::CONFLICT,
                msg: format!("model-set mismatch: server serves '{server}', request expects '{req}'"),
            });
        }
    }
    Ok(())
}

/// Take a concurrency permit, or 503 if the cap is reached. `None` when no cap
/// is configured. The returned permit must be held for the request's lifetime.
fn acquire(st: &AppState) -> Result<Option<OwnedSemaphorePermit>, ApiError> {
    match &st.0.semaphore {
        Some(sem) => match sem.clone().try_acquire_owned() {
            Ok(p) => Ok(Some(p)),
            Err(_) => {
                st.0.metrics.busy.fetch_add(1, Ordering::Relaxed);
                Err(ApiError {
                    status: StatusCode::SERVICE_UNAVAILABLE,
                    msg: "server busy: concurrency limit reached".into(),
                })
            }
        },
        None => Ok(None),
    }
}

fn maybe_log(st: &AppState, endpoint: &str, text: &str) {
    if st.0.log_content {
        eprintln!("drishti-server[content] {endpoint}: {text:?}");
    }
}

/// Run a core check on the blocking pool, bounded by the per-request timeout.
/// A timeout is surfaced as 504 (the blocking task is left to finish; the client
/// gets a definite non-verdict and can fail closed).
async fn run_guarded<T, F>(st: &AppState, f: F) -> Result<T, ApiError>
where
    F: FnOnce() -> Result<T, DrishtiError> + Send + 'static,
    T: Send + 'static,
{
    let dur = Duration::from_millis(st.0.request_timeout_ms);
    let handle = tokio::task::spawn_blocking(f);
    match tokio::time::timeout(dur, handle).await {
        Ok(Ok(r)) => r.map_err(map_err),
        Ok(Err(join)) => Err(ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            msg: format!("task join failed: {join}"),
        }),
        Err(_) => Err(ApiError {
            status: StatusCode::GATEWAY_TIMEOUT,
            msg: "request timed out".into(),
        }),
    }
}

async fn check_prompt(
    State(st): State<AppState>,
    Json(req): Json<PromptReq>,
) -> Result<Json<PromptCheck>, ApiError> {
    let server_ms = require_loaded(&st).map_err(|e| record_err(&st, e))?.model_set.clone();
    check_model_set(&req.model_set, &server_ms).map_err(|e| record_err(&st, e))?;
    let _permit = acquire(&st)?;
    st.0.metrics.prompt.fetch_add(1, Ordering::Relaxed);
    maybe_log(&st, "prompt", &req.input);
    let st2 = st.0.clone();
    let result = run_guarded(&st, move || {
        let engine = &st2.loaded.get().expect("engine loaded").engine;
        futures::executor::block_on(engine.check_prompt(&req.input))
    })
    .await
    .map_err(|e| record_err(&st, e))?;
    st.0.metrics
        .latency_ms_sum
        .fetch_add(result.latency_ms as u64, Ordering::Relaxed);
    Ok(Json(result))
}

async fn check_pii(
    State(st): State<AppState>,
    Json(req): Json<PiiReq>,
) -> Result<Json<PiiCheck>, ApiError> {
    let server_ms = require_loaded(&st).map_err(|e| record_err(&st, e))?.model_set.clone();
    check_model_set(&req.model_set, &server_ms).map_err(|e| record_err(&st, e))?;
    let _permit = acquire(&st)?;
    st.0.metrics.pii.fetch_add(1, Ordering::Relaxed);
    maybe_log(&st, "pii", &req.input);
    let st2 = st.0.clone();
    let result = run_guarded(&st, move || {
        let engine = &st2.loaded.get().expect("engine loaded").engine;
        futures::executor::block_on(engine.check_pii(&req.input))
    })
    .await
    .map_err(|e| record_err(&st, e))?;
    st.0.metrics
        .latency_ms_sum
        .fetch_add(result.latency_ms as u64, Ordering::Relaxed);
    Ok(Json(result))
}

async fn check_output(
    State(st): State<AppState>,
    Json(req): Json<OutputReq>,
) -> Result<Json<OutputCheck>, ApiError> {
    let server_ms = require_loaded(&st).map_err(|e| record_err(&st, e))?.model_set.clone();
    check_model_set(&req.model_set, &server_ms).map_err(|e| record_err(&st, e))?;
    let _permit = acquire(&st)?;
    st.0.metrics.output.fetch_add(1, Ordering::Relaxed);
    maybe_log(&st, "output", &req.output);
    let st2 = st.0.clone();
    let result = run_guarded(&st, move || {
        let engine = &st2.loaded.get().expect("engine loaded").engine;
        futures::executor::block_on(engine.check_output(&req.output))
    })
    .await
    .map_err(|e| record_err(&st, e))?;
    st.0.metrics
        .latency_ms_sum
        .fetch_add(result.latency_ms as u64, Ordering::Relaxed);
    Ok(Json(result))
}

async fn check_all(
    State(st): State<AppState>,
    Json(req): Json<AllReq>,
) -> Result<Json<FullCheck>, ApiError> {
    let server_ms = require_loaded(&st).map_err(|e| record_err(&st, e))?.model_set.clone();
    check_model_set(&req.model_set, &server_ms).map_err(|e| record_err(&st, e))?;
    let _permit = acquire(&st)?;
    st.0.metrics.all.fetch_add(1, Ordering::Relaxed);
    let st2 = st.0.clone();
    let result = run_guarded(&st, move || {
        let engine = &st2.loaded.get().expect("engine loaded").engine;
        futures::executor::block_on(engine.check_all(&req.prompt, req.output.as_deref()))
    })
    .await
    .map_err(|e| record_err(&st, e))?;
    Ok(Json(result))
}

/// 413 when a batch exceeds the configured cap.
fn check_batch_size(st: &AppState, n: usize) -> Result<(), ApiError> {
    if n > st.0.max_batch {
        return Err(ApiError {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            msg: format!("batch of {n} exceeds the max of {}", st.0.max_batch),
        });
    }
    Ok(())
}

async fn check_prompt_batch(
    State(st): State<AppState>,
    Json(req): Json<PromptBatchReq>,
) -> Result<Json<Vec<PromptCheck>>, ApiError> {
    let server_ms = require_loaded(&st).map_err(|e| record_err(&st, e))?.model_set.clone();
    check_model_set(&req.model_set, &server_ms).map_err(|e| record_err(&st, e))?;
    check_batch_size(&st, req.inputs.len()).map_err(|e| record_err(&st, e))?;
    let _permit = acquire(&st)?;
    st.0.metrics
        .prompt
        .fetch_add(req.inputs.len() as u64, Ordering::Relaxed);
    let st2 = st.0.clone();
    let result = run_guarded(&st, move || {
        let engine = &st2.loaded.get().expect("engine loaded").engine;
        let refs: Vec<&str> = req.inputs.iter().map(|s| s.as_str()).collect();
        futures::executor::block_on(engine.check_prompt_batch(&refs))
    })
    .await
    .map_err(|e| record_err(&st, e))?;
    Ok(Json(result))
}

async fn check_pii_batch(
    State(st): State<AppState>,
    Json(req): Json<PiiBatchReq>,
) -> Result<Json<Vec<PiiCheck>>, ApiError> {
    let server_ms = require_loaded(&st).map_err(|e| record_err(&st, e))?.model_set.clone();
    check_model_set(&req.model_set, &server_ms).map_err(|e| record_err(&st, e))?;
    check_batch_size(&st, req.inputs.len()).map_err(|e| record_err(&st, e))?;
    let _permit = acquire(&st)?;
    st.0.metrics
        .pii
        .fetch_add(req.inputs.len() as u64, Ordering::Relaxed);
    let st2 = st.0.clone();
    let result = run_guarded(&st, move || {
        let engine = &st2.loaded.get().expect("engine loaded").engine;
        let refs: Vec<&str> = req.inputs.iter().map(|s| s.as_str()).collect();
        futures::executor::block_on(engine.check_pii_batch(&refs))
    })
    .await
    .map_err(|e| record_err(&st, e))?;
    Ok(Json(result))
}

async fn check_output_batch(
    State(st): State<AppState>,
    Json(req): Json<OutputBatchReq>,
) -> Result<Json<Vec<OutputCheck>>, ApiError> {
    let server_ms = require_loaded(&st).map_err(|e| record_err(&st, e))?.model_set.clone();
    check_model_set(&req.model_set, &server_ms).map_err(|e| record_err(&st, e))?;
    check_batch_size(&st, req.outputs.len()).map_err(|e| record_err(&st, e))?;
    let _permit = acquire(&st)?;
    st.0.metrics
        .output
        .fetch_add(req.outputs.len() as u64, Ordering::Relaxed);
    let st2 = st.0.clone();
    let result = run_guarded(&st, move || {
        let engine = &st2.loaded.get().expect("engine loaded").engine;
        let refs: Vec<&str> = req.outputs.iter().map(|s| s.as_str()).collect();
        futures::executor::block_on(engine.check_output_batch(&refs))
    })
    .await
    .map_err(|e| record_err(&st, e))?;
    Ok(Json(result))
}

async fn manifest(State(st): State<AppState>) -> Result<Json<ModelManifest>, ApiError> {
    let loaded = require_loaded(&st)?;
    Ok(Json(loaded.engine.model_manifest()))
}

async fn readyz(State(st): State<AppState>) -> Response {
    if st.0.loaded.get().is_some() {
        (StatusCode::OK, "ready").into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "loading").into_response()
    }
}

/// Open endpoint carrying the model-set identity and Drishti version, so an
/// orchestrator or a client can confirm what it is talking to. No content.
async fn version(State(st): State<AppState>) -> Response {
    let model_set = st.0.loaded.get().map(|l| l.model_set.clone());
    Json(json!({
        "drishti_version": DRISHTI_VERSION,
        "model_set": model_set,
        "ready": st.0.loaded.get().is_some(),
    }))
    .into_response()
}

async fn metrics_handler(State(st): State<AppState>) -> impl IntoResponse {
    let loaded = st.0.loaded.get();
    let body = st
        .0
        .metrics
        .render(loaded.is_some(), loaded.map(|l| l.model_set.as_str()));
    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], body)
}

/// Bearer-token gate on the check and manifest endpoints. Health, readiness,
/// version, and metrics stay open.
async fn auth(State(st): State<AppState>, req: Request, next: Next) -> Result<Response, ApiError> {
    if let Some(expected) = &st.0.token {
        let presented = req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "));
        if presented != Some(expected.as_str()) {
            return Err(ApiError {
                status: StatusCode::UNAUTHORIZED,
                msg: "missing or invalid bearer token".into(),
            });
        }
    }
    Ok(next.run(req).await)
}

/// A stable id for the loaded model set: a short hash over each model's role, id,
/// and pinned sha256 plus the regex version. Two servers with the same models
/// produce the same id; any model change produces a different one. See ADR-012.
fn derive_model_set(m: &ModelManifest) -> String {
    let mut lines: Vec<String> = m
        .models
        .iter()
        .map(|e| format!("{}|{}|{}", e.role, e.model_id, e.sha256))
        .collect();
    lines.sort();
    lines.push(format!("regex|{}", m.regex_version));
    let mut hasher = Sha256::new();
    hasher.update(lines.join("\n").as_bytes());
    format!("ms-{}", &hex::encode(hasher.finalize())[..16])
}

fn build_router(state: AppState, max_request_bytes: usize) -> Router {
    let protected = Router::new()
        .route("/v1/check/prompt", post(check_prompt))
        .route("/v1/check/pii", post(check_pii))
        .route("/v1/check/output", post(check_output))
        .route("/v1/check/all", post(check_all))
        .route("/v1/check/prompt/batch", post(check_prompt_batch))
        .route("/v1/check/pii/batch", post(check_pii_batch))
        .route("/v1/check/output/batch", post(check_output_batch))
        .route("/v1/manifest", get(manifest))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth))
        .layer(DefaultBodyLimit::max(max_request_bytes));

    let open = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(readyz))
        .route("/v1/version", get(version))
        .route("/metrics", get(metrics_handler));

    protected.merge(open).with_state(state)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    dotenvy::dotenv().ok();
    let config_text = std::fs::read_to_string(&args.config)?;
    let drishti_config = DrishtiConfig::from_toml_and_env(&config_text)?;
    let server_config = ServerConfig::load(&config_text);

    let token = args.token.or_else(|| std::env::var("DRISHTI_TOKEN").ok());
    if token.is_none() {
        eprintln!("warning: no bearer token set; check endpoints are unauthenticated");
    }
    if server_config.log_content {
        eprintln!(
            "WARNING: [server].log_content is ON. Request and response CONTENT (which can carry \
             end-user PII) will be written to logs. Use for local development only."
        );
    }

    let semaphore = server_config
        .max_concurrency
        .map(|n| Arc::new(Semaphore::new(n.max(1))));
    let request_timeout_ms = server_config
        .request_timeout_ms
        .unwrap_or(DEFAULT_REQUEST_TIMEOUT_MS);
    let max_batch = server_config.max_batch.unwrap_or(DEFAULT_MAX_BATCH);
    let max_request_bytes = server_config
        .max_request_bytes
        .unwrap_or(DEFAULT_MAX_REQUEST_BYTES);

    let state = AppState(Arc::new(Inner {
        loaded: OnceLock::new(),
        token,
        metrics: Metrics::default(),
        semaphore,
        request_timeout_ms,
        max_batch,
        log_content: server_config.log_content,
    }));

    // Load models in the background so the socket answers /healthz and /readyz
    // immediately; /readyz stays false until the (eager, sha256-verified) load
    // completes. A load failure is fatal and exits the process so an orchestrator
    // restarts it, preserving today's fail-loud-on-bad-config behaviour.
    let loader_state = state.clone();
    let cache_dir = drishti_config.cache_dir.clone();
    let operator_model_set = server_config.model_set.clone();
    tokio::task::spawn_blocking(move || {
        let source = FsSource::with_optional_cache(cache_dir);
        match Drishti::builder().with_config(drishti_config).build(&source) {
            Ok(engine) => {
                let model_set = operator_model_set
                    .unwrap_or_else(|| derive_model_set(&engine.model_manifest()));
                eprintln!("drishti-server: models loaded, model_set={model_set}, ready");
                let _ = loader_state.0.loaded.set(Loaded { engine, model_set });
            }
            Err(e) => {
                eprintln!("drishti-server: FATAL model load failed: {e}");
                std::process::exit(1);
            }
        }
    });

    let app = build_router(state, max_request_bytes);
    let addr: SocketAddr = args.bind.parse()?;

    match (&server_config.tls_cert_path, &server_config.tls_key_path) {
        (Some(cert), Some(key)) => {
            // rustls 0.23 needs a process-wide crypto provider installed before
            // any TLS config is built.
            let _ = rustls::crypto::ring::default_provider().install_default();
            let tls = RustlsConfig::from_pem_file(cert, key).await?;
            println!("drishti-server listening on https://{addr}");
            axum_server::bind_rustls(addr, tls)
                .serve(app.into_make_service())
                .await?;
        }
        (None, None) => {
            println!("drishti-server listening on http://{addr}");
            axum_server::bind(addr).serve(app.into_make_service()).await?;
        }
        _ => {
            return Err("TLS requires both [server].tls_cert_path and tls_key_path".into());
        }
    }
    Ok(())
}
