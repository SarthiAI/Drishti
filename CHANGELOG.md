# Changelog

All notable changes to Drishti are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project aims to
follow semantic versioning once it reaches 1.0.

## [Unreleased]

### Added

- GPU execution and remote serving, all additive and opt-in; the embedded,
  in-process, CPU path stays the default and is unchanged (see [SERVING.md](SERVING.md)):
  - An `execution_provider` config option (`cpu` default, `cuda`, `tensorrt`,
    `auto`) plus `gpu_device_id`, so the ONNX classifiers can run on a GPU.
    Enabled in a build made with `--features cuda` or `--features tensorrt`;
    `auto` falls back to the CPU, and an explicit GPU choice on a CPU-only build
    fails closed with a clear error.
  - A backend-agnostic `SafetyEngine` trait in `drishti-core`, implemented by
    both the embedded engine and a new remote client, so a host holds one handle
    regardless of where inference runs. A distinct `SafetyError` separates a
    verdict-time fault from an unreachable backend so hosts can fail closed.
  - A new `sarthiai-drishti-client` crate: a Rust `RemoteDrishti` that speaks to
    a `drishti-server` over authenticated, optionally TLS, JSON.
  - Server hardening (opt-in via a `[server]` config table): optional TLS, an
    optional pinned model-set id with HTTP 409 on mismatch, a request-body limit
    (413), a per-request timeout (504), a concurrency cap that sheds with 503,
    `/batch` endpoints, an honest `/readyz` (false while models load), a
    `/v1/version` endpoint, and a debug-only content-logging flag (off, warned).
    Request and response content is never logged by default.

- Three content-safety checks: prompt-injection detection, PII detection and
  redaction, and output-safety classification.
- Three surfaces over one core: the `drishti` CLI, the `drishti-server` HTTP
  service, and the `drishti` Python package, all returning identical results.
- Configurable models with present-or-fetch loading: nothing hardcoded, download
  on first use, optional SHA-256 verification, bring-your-own-model by local path.
- Configuration via TOML with environment-variable and `.env` overrides for every
  value.
- PII regex stage (emails, cards with Luhn, phones, IPs, IBAN, SSN, PAN, Aadhaar,
  UPI, NINO, VAT) and an optional NER stage with an acronym filter.
- Output-safety support for both multi-label and softmax-with-safe-class models.
- The `drishti-eval` harness: precision, recall, and F1 against labelled sets,
  with a JSON report stamped with model hashes, and a validated-versus-experimental
  gate.
- A ready-to-run starter configuration and a reference Docker image.
- Elastic License 2.0.

[Unreleased]: https://github.com/sarthiai/drishti
