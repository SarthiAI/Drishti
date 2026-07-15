//! `drishti-client`: a remote client for a running `drishti-server`. It
//! implements [`drishti_core::SafetyEngine`], so a host swaps embedded inference
//! for a shared GPU service by constructing a [`RemoteDrishti`] instead of a
//! `Drishti`; the calling code is unchanged.
//!
//! The load-bearing contract is the error split. Every transport failure,
//! timeout, non-2xx response (including a `409` model-set mismatch), or decode
//! error becomes [`SafetyError::Unavailable`], never a verdict. A host is
//! expected to fail **closed** on `Unavailable`: block the traffic rather than
//! let an ungoverned call through because the safety service was unreachable.
//!
//! Security note: request text can contain end-user PII and prompts (that is the
//! point of the PII check), and redaction now happens on the service. Point a
//! `RemoteDrishti` only at a service inside your own trust boundary, over TLS,
//! with a bearer token. Never across the public internet.

use std::time::Duration;

use async_trait::async_trait;
use drishti_core::error::SafetyError;
use drishti_core::{OutputCheck, PiiCheck, PromptCheck, SafetyEngine};
use serde::de::DeserializeOwned;
use serde::Serialize;

/// How to reach a `drishti-server`. Everything except `base_url` has a sensible
/// default via [`RemoteConfig::new`], and every field is meant to be driven from
/// the host's own configuration, not hardcoded.
#[derive(Clone, Debug)]
pub struct RemoteConfig {
    /// Base URL of the service, e.g. `https://drishti.internal:8443`.
    pub base_url: String,
    /// Bearer token presented on every check call. `None` only for an
    /// unauthenticated service (local dev).
    pub auth_token: Option<String>,
    /// The model-set id this host expects. When set, it is sent on every request
    /// and the server returns `409` (surfaced as `Unavailable`) if it serves a
    /// different set, so a server on a different model version cannot silently
    /// change decisions. `None` sends no pin and accepts whatever is loaded.
    pub model_set: Option<String>,
    /// TCP connect timeout.
    pub connect_timeout: Duration,
    /// Total per-request timeout (connect + response). A timeout is `Unavailable`.
    pub request_timeout: Duration,
    /// A PEM-encoded CA certificate to trust, for a service presenting a
    /// private/self-signed certificate inside the tenant. `None` uses the system
    /// roots.
    pub tls_ca_pem: Option<String>,
    /// Accept any server certificate. In-tenant convenience for self-signed
    /// setups only; document loudly and prefer `tls_ca_pem` in production.
    pub danger_accept_invalid_certs: bool,
}

impl RemoteConfig {
    /// A config pointing at `base_url` with 2s connect / 10s request timeouts,
    /// no auth, no pin, system TLS roots. Fill in the rest as needed.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            auth_token: None,
            model_set: None,
            connect_timeout: Duration::from_secs(2),
            request_timeout: Duration::from_secs(10),
            tls_ca_pem: None,
            danger_accept_invalid_certs: false,
        }
    }

    /// Set the bearer token.
    pub fn with_auth(mut self, token: impl Into<String>) -> Self {
        self.auth_token = Some(token.into());
        self
    }

    /// Set the expected model-set id (sent on every request for the `409` check).
    pub fn with_model_set(mut self, id: impl Into<String>) -> Self {
        self.model_set = Some(id.into());
        self
    }
}

/// A handle to a remote Drishti service. Cheap to clone (wraps a connection-
/// pooling [`reqwest::Client`]).
#[derive(Clone)]
pub struct RemoteDrishti {
    http: reqwest::Client,
    base_url: String,
    auth_token: Option<String>,
    model_set: Option<String>,
}

impl RemoteDrishti {
    /// Build a client from a [`RemoteConfig`]. Constructing the HTTP client can
    /// fail on a bad TLS CA or an invalid setup; that surfaces as
    /// `Unavailable` so the host can treat a misconfigured client the same way
    /// it treats an unreachable server (fail closed).
    pub fn connect(cfg: RemoteConfig) -> Result<Self, SafetyError> {
        let mut builder = reqwest::Client::builder()
            .connect_timeout(cfg.connect_timeout)
            .timeout(cfg.request_timeout);

        if let Some(ca) = &cfg.tls_ca_pem {
            let cert = reqwest::Certificate::from_pem(ca.as_bytes())
                .map_err(|e| SafetyError::Unavailable(format!("invalid TLS CA pem: {e}")))?;
            builder = builder.add_root_certificate(cert);
        }
        if cfg.danger_accept_invalid_certs {
            builder = builder.danger_accept_invalid_certs(true);
        }

        let http = builder
            .build()
            .map_err(|e| SafetyError::Unavailable(format!("build HTTP client: {e}")))?;

        Ok(Self {
            http,
            base_url: cfg.base_url.trim_end_matches('/').to_string(),
            auth_token: cfg.auth_token,
            model_set: cfg.model_set,
        })
    }

    /// The model-set id this client pins, if any.
    pub fn model_set(&self) -> Option<&str> {
        self.model_set.as_deref()
    }

    async fn post<B, R>(&self, path: &str, body: &B) -> Result<R, SafetyError>
    where
        B: Serialize,
        R: DeserializeOwned,
    {
        let url = format!("{}{}", self.base_url, path);
        let mut req = self.http.post(&url).json(body);
        if let Some(token) = &self.auth_token {
            req = req.bearer_auth(token);
        }

        let resp = req.send().await.map_err(|e| {
            // Connect refused, DNS failure, TLS failure, or a timeout: the
            // backend is not usable. Fail closed.
            SafetyError::Unavailable(format!("request to {url} failed: {e}"))
        })?;

        let status = resp.status();
        if !status.is_success() {
            // Every non-2xx is unavailability from the host's point of view: a
            // 401 (auth), 409 (model-set mismatch), 413 (too large), 503 (busy),
            // or 504 (timeout) all mean "no trustworthy verdict", so fail closed.
            let detail = resp.text().await.unwrap_or_default();
            let detail = detail.trim();
            return Err(SafetyError::Unavailable(if detail.is_empty() {
                format!("{url} returned {status}")
            } else {
                format!("{url} returned {status}: {detail}")
            }));
        }

        resp.json::<R>()
            .await
            .map_err(|e| SafetyError::Unavailable(format!("decoding {url} response failed: {e}")))
    }
}

/// Request bodies. Field names match the server contract exactly: `input` for
/// prompt and pii, `output` for output. `model_set` is sent only when pinned.
#[derive(Serialize)]
struct InputReq<'a> {
    input: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_set: Option<&'a str>,
}

#[derive(Serialize)]
struct OutputReq<'a> {
    output: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_set: Option<&'a str>,
}

#[async_trait]
impl SafetyEngine for RemoteDrishti {
    async fn check_prompt(&self, text: &str) -> Result<PromptCheck, SafetyError> {
        self.post(
            "/v1/check/prompt",
            &InputReq {
                input: text,
                model_set: self.model_set.as_deref(),
            },
        )
        .await
    }

    async fn check_pii(&self, text: &str) -> Result<PiiCheck, SafetyError> {
        self.post(
            "/v1/check/pii",
            &InputReq {
                input: text,
                model_set: self.model_set.as_deref(),
            },
        )
        .await
    }

    async fn check_output(&self, text: &str) -> Result<OutputCheck, SafetyError> {
        self.post(
            "/v1/check/output",
            &OutputReq {
                output: text,
                model_set: self.model_set.as_deref(),
            },
        )
        .await
    }
}
