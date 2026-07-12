//! `drishti-server`: the HTTP service. A thin adapter over `drishti-core`
//! (invariant I5): every endpoint deserializes a request, calls the same core
//! method the CLI calls, and serializes the result. No detection logic lives
//! here. Inference is blocking CPU work, so it runs on Tokio's blocking pool to
//! keep the async reactor free.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use drishti_core::config::DrishtiConfig;
use drishti_core::error::DrishtiError;
use drishti_core::{Drishti, FullCheck, ModelManifest, OutputCheck, PiiCheck, PromptCheck};
use drishti_models::FsSource;
use serde::Deserialize;
use serde_json::json;

#[derive(Parser)]
#[command(name = "drishti-server", about = "Drishti content-safety HTTP service")]
struct Args {
    /// TOML config selecting the models for each check (same schema as the CLI).
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

#[derive(Clone)]
struct AppState(Arc<Inner>);

struct Inner {
    drishti: Drishti,
    token: Option<String>,
    metrics: Metrics,
}

#[derive(Default)]
struct Metrics {
    prompt: AtomicU64,
    pii: AtomicU64,
    output: AtomicU64,
    all: AtomicU64,
    errors: AtomicU64,
    latency_ms_sum: AtomicU64,
}

impl Metrics {
    fn render(&self) -> String {
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
            "drishti_latency_ms_sum",
            "Cumulative reported check latency in milliseconds.",
            &[("", self.latency_ms_sum.load(Ordering::Relaxed))],
        );
        s
    }
}

// Request bodies. PII redaction policy is server configuration (set at build),
// not a per-request field, so the request is just the input. Per-request
// redaction override is recorded as backlog.
#[derive(Deserialize)]
struct PromptReq {
    input: String,
}
#[derive(Deserialize)]
struct PiiReq {
    input: String,
}
#[derive(Deserialize)]
struct OutputReq {
    output: String,
}
#[derive(Deserialize)]
struct AllReq {
    prompt: String,
    #[serde(default)]
    output: Option<String>,
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

/// Run a core check on the blocking pool so inference never stalls the reactor.
async fn run_blocking<T, F>(f: F) -> Result<T, ApiError>
where
    F: FnOnce() -> Result<T, DrishtiError> + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(r) => r.map_err(map_err),
        Err(e) => Err(ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            msg: format!("task join failed: {e}"),
        }),
    }
}

fn record_err(st: &AppState, e: ApiError) -> ApiError {
    st.0.metrics.errors.fetch_add(1, Ordering::Relaxed);
    e
}

async fn check_prompt(
    State(st): State<AppState>,
    Json(req): Json<PromptReq>,
) -> Result<Json<PromptCheck>, ApiError> {
    st.0.metrics.prompt.fetch_add(1, Ordering::Relaxed);
    let inner = st.0.clone();
    let result = run_blocking(move || futures::executor::block_on(inner.drishti.check_prompt(&req.input)))
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
    st.0.metrics.pii.fetch_add(1, Ordering::Relaxed);
    let inner = st.0.clone();
    let result = run_blocking(move || futures::executor::block_on(inner.drishti.check_pii(&req.input)))
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
    st.0.metrics.output.fetch_add(1, Ordering::Relaxed);
    let inner = st.0.clone();
    let result = run_blocking(move || futures::executor::block_on(inner.drishti.check_output(&req.output)))
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
    st.0.metrics.all.fetch_add(1, Ordering::Relaxed);
    let inner = st.0.clone();
    let result = run_blocking(move || {
        futures::executor::block_on(inner.drishti.check_all(&req.prompt, req.output.as_deref()))
    })
    .await
    .map_err(|e| record_err(&st, e))?;
    Ok(Json(result))
}

async fn manifest(State(st): State<AppState>) -> Json<ModelManifest> {
    Json(st.0.drishti.model_manifest())
}

async fn metrics_handler(State(st): State<AppState>) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        st.0.metrics.render(),
    )
}

/// Bearer-token gate on the check and manifest endpoints. Health and metrics
/// stay open.
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    dotenvy::dotenv().ok();
    let config_text = std::fs::read_to_string(&args.config)?;
    let config = DrishtiConfig::from_toml_and_env(&config_text)?;
    let source = FsSource::with_optional_cache(config.cache_dir.clone());
    let drishti = Drishti::builder().with_config(config).build(&source)?;

    let token = args.token.or_else(|| std::env::var("DRISHTI_TOKEN").ok());
    if token.is_none() {
        eprintln!("warning: no bearer token set; check endpoints are unauthenticated");
    }

    let state = AppState(Arc::new(Inner {
        drishti,
        token,
        metrics: Metrics::default(),
    }));

    let protected = Router::new()
        .route("/v1/check/prompt", post(check_prompt))
        .route("/v1/check/pii", post(check_pii))
        .route("/v1/check/output", post(check_output))
        .route("/v1/check/all", post(check_all))
        .route("/v1/manifest", get(manifest))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth));

    let open = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(|| async { "ready" }))
        .route("/metrics", get(metrics_handler));

    let app = protected.merge(open).with_state(state);

    let addr: SocketAddr = args.bind.parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("drishti-server listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
