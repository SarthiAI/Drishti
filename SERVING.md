# Running Drishti: embedded, GPU, or as a shared service

Drishti runs three checks (prompt injection, PII, output safety) around your LLM
calls. How it runs is a deployment choice. All three options below use the same
models, the same thresholds, and return the same decisions. Nothing here changes
the default: if you do nothing, Drishti runs embedded, in-process, on the CPU,
exactly as before.

## The three options

1. Embedded, CPU (the default). Simplest, one process, fine for low call rates.
2. Embedded, GPU. Same in-process code, running the classifiers on a GPU on the
   gateway machine.
3. Separate service, GPU. One GPU-backed `drishti-server` that many small, cheap
   CPU gateways call over the network. This lets you scale "how fast is safety"
   independently of "how many gateways".

## Option 2: run the classifiers on a GPU

Two ONNX classifiers (prompt injection and output moderation) dominate the cost.
A GPU runs them much faster and can batch them. To use one:

1. Build with a GPU execution provider. The default build is CPU-only on purpose,
   so it stays portable. A GPU build is opt-in:

   ```bash
   # NVIDIA CUDA
   cargo build --release -p drishti-server --features cuda
   # or TensorRT
   cargo build --release -p drishti-server --features tensorrt
   ```

2. Provide a GPU-enabled ONNX Runtime. Drishti loads ONNX Runtime at runtime from
   `ORT_DYLIB_PATH` (it does not bundle it). Point that at a CUDA-enabled
   `libonnxruntime` that matches ONNX Runtime 1.24.x. The CPU and GPU builds of
   ONNX Runtime are different downloads.

3. Choose the provider in config (or by environment variable):

   ```toml
   execution_provider = "cuda"   # "cpu" (default) | "cuda" | "tensorrt" | "auto"
   gpu_device_id = 0
   ```

   or `DRISHTI_EXECUTION_PROVIDER=cuda`. `auto` tries a GPU and falls back to the
   CPU, logging which it used. An explicit `cuda` or `tensorrt` on a build or a
   machine without it fails to start with a clear error, so a misconfigured node
   fails closed instead of silently running slow on the CPU.

A note on numbers: the speed-up is large and it is real. On an NVIDIA RTX A5000,
the heavy DeBERTa prompt-injection model runs at about 110 checks per second
(median 72 ms) on the GPU against about 2.2 checks per second (median 3.5 s) on
the CPU, roughly a 50x improvement. Your exact numbers depend on your GPU, the
model, and the batch size, so measure on your hardware. The decisions do not
change: GPU and CPU return the same block-or-mask outcome, with raw scores that
differ only in the last digits.

## Option 3: run Drishti as a separate service

Start `drishti-server` with the same config the embedded engine uses (same
models, same thresholds, and the same `execution_provider` option, so the service
itself can be on a GPU):

```bash
drishti-server --config config/serving-example.toml --bind 0.0.0.0:8443 --token "$DRISHTI_TOKEN"
```

The service binds and answers `/healthz` and `/readyz` immediately, and loads
models in the background. `/readyz` returns 503 until the models are up and 200
after, so a load balancer only sends traffic once the service is ready. Point a
gateway at the service with a client (below).

### Server settings (all optional)

These live in an optional `[server]` table in the same config file, and each is
also settable by environment variable as `DRISHTI_SERVER__<NAME>`. See
[config/serving-example.toml](config/serving-example.toml) for a full example.

| Setting | Meaning | Default |
|---|---|---|
| `tls_cert_path` + `tls_key_path` | Serve HTTPS when both are set | plain HTTP |
| `max_request_bytes` | Reject larger bodies with 413 | 1 MiB |
| `request_timeout_ms` | Time out a request with 504 | 30000 |
| `max_concurrency` | Shed extra in-flight requests with 503 | unlimited |
| `max_batch` | Max items a `/batch` endpoint accepts (413 above) | 32 |
| `model_set` | Id clients pin; a mismatch returns 409 | derived from the models |
| `log_content` | Log request and response content (debug only) | off |

Security: the request text can contain the end user's PII and prompts, and PII
redaction now happens on the service, so raw content travels from the gateway to
the service. Run the service self-hosted, inside your own trust boundary only,
over TLS with a bearer token. Never expose it outside the enterprise boundary.
Content is never written to logs unless `log_content` is turned on, which is for
local development only and is loudly warned.

### Model-set identity

Clients can pin the exact model set they expect by sending `model_set` on each
request. The server compares it and returns 409 on a mismatch, so a server that
was moved to a different model version cannot silently change your decisions. The
id is either the `[server].model_set` you set, or one derived from the loaded
models (a hash of each model's id and sha256). Read the current id from
`GET /v1/version`.

## Calling the service from Rust

The `sarthiai-drishti-client` crate provides `RemoteDrishti`, which implements the
same `SafetyEngine` trait as the embedded `Drishti`. Your calling code does not
change; you only choose which one to construct:

```rust
use drishti_client::{RemoteConfig, RemoteDrishti};
use drishti_core::SafetyEngine;

let engine = RemoteDrishti::connect(
    RemoteConfig::new("https://drishti.internal:8443")
        .with_auth(std::env::var("DRISHTI_TOKEN")?)
        .with_model_set("niyam-default-v1"),
)?;

match engine.check_pii("email a@b.com").await {
    Ok(check) => { /* use check.refuse, check.redacted, check.spans */ }
    Err(e) => { /* SafetyError::Unavailable: the service is unreachable; FAIL CLOSED (block) */ }
}
```

The important contract: any transport failure, timeout, non-2xx response, or
model-set mismatch is returned as `SafetyError::Unavailable`, which is distinct
from a verdict. A host must fail closed on it, that is block the traffic, rather
than let an ungoverned call through because the safety service was unreachable.

Python and Node remote SDKs (`sarthiai-drishti-sdk`) are in
[clients/](clients/) and follow the same fail-closed convention.

## Moving from embedded to remote does not change decisions

For the same input, the same models, and the same thresholds, the remote path
returns the same block-or-mask decision as the embedded path. A parity harness
([../client-e2e/parity-rust.sh](../client-e2e/parity-rust.sh)) proves this by
running a fixed corpus through both and asserting the decisions and the redacted
strings match.
