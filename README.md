<div align="center">

# दृष्टि Drishti

**Fast, honest, self-hostable content safety for LLM systems.**

Prompt-injection detection, PII detection and redaction, and output-safety
classification. Three checks, one package, running entirely on your hardware.

![License](https://img.shields.io/badge/license-Elastic--2.0-2f6f4e)
![Built with Rust](https://img.shields.io/badge/built%20with-Rust-dea584?logo=rust&logoColor=white)
![Python](https://img.shields.io/badge/python-3.9%20to%203.13-3776ab?logo=python&logoColor=white)
![Platforms](https://img.shields.io/badge/platforms-Linux%20%7C%20macOS%20%7C%20Windows-555)
![Inference](https://img.shields.io/badge/inference-ONNX%20Runtime-005CED)
![Scores](https://img.shields.io/badge/it%20scores-it%20does%20not%20decide-6f42c1)

</div>

---

Drishti (दृष्टि, "sight") watches the text that flows in and out of LLM systems
and reports what it sees. It runs three checks:

| Check | On | Returns |
|---|---|---|
| **Prompt injection** | inputs | an injection score, a class, and a confidence |
| **PII detection and redaction** | inputs | located PII spans plus a redacted copy of the text |
| **Output safety** | outputs | a score per safety category and an aggregate verdict |

Every check returns a calibrated score and the identity of the model that
produced it. Drishti never makes a policy decision on its own. It sees, it
scores, and it lets your policy layer decide.

### Highlights

- **Three checks, one package.** No assembling three Python projects with three
  runtimes and three opinions about the output type.
- **Scores, not verdicts.** Every result is a number a deterministic policy can
  act on. Drishti refuses nothing itself.
- **Models are yours.** Nothing is hardcoded. You choose the model per check; if
  it is missing, Drishti downloads it once, verifies it, and caches it.
- **Offline by default.** No default code path calls a remote service. CPU first,
  GPU optional.
- **Honest numbers.** Precision and recall come from the eval harness, and any
  path that has not cleared its bar is labelled experimental.
- **Three surfaces, one core.** A Rust crate, a Python package, and an HTTP
  service that all return identical results.

---

## Table of contents

- [Install](#install)
- [Quick start](#quick-start)
- [The three checks](#the-three-checks)
- [Configuration](#configuration)
- [Models](#models)
- [HTTP API](#http-api)
- [Eval results](#eval-results)
- [Performance](#performance)
- [Threat model](#threat-model)
- [Project layout](#project-layout)
- [License](#license)
- [Part of Niyam](#part-of-niyam)

---

## Install

Drishti is approaching its first tagged release. Published wheels and images will
be available from the channels below; until then, build from source.

**Python** (embedded, runs models in-process; imported as `drishti`)

```bash
pip install sarthiai-drishti
```

One abi3 wheel per platform covers Python 3.9 through 3.13, on Linux x86_64,
Linux ARM64, macOS ARM64 (Apple Silicon), and Windows x86_64.

**Remote client SDKs** (call a running `drishti-server`, no model loaded locally)

```bash
pip install sarthiai-drishti-sdk        # Python, imported as drishti_sdk
npm install sarthiai-drishti-sdk        # Node
```

**Docker**

```bash
docker pull sarthiai/drishti
```

Multi-architecture (linux/amd64 and linux/arm64). The container carries its own
Linux, so the host operating system does not matter.

**From source**

```bash
git clone https://github.com/SarthiAI/Drishti
cd Drishti
cargo build --release          # builds the CLI and the server
pip install maturin && maturin develop --release   # builds the Python wheel
```

**ONNX Runtime at runtime.** Drishti links ONNX Runtime dynamically (ort's
`load-dynamic`), so the build is pure Rust and the artifacts stay small and
portable. The library is provided at runtime:

- The `sarthiai-drishti` wheel and the Docker image pull it in automatically (a
  dependency of the wheel; downloaded into the image), so `pip install` and
  `docker run` just work.
- Running the CLI or server from a source build: install ONNX Runtime (for
  example `pip install "onnxruntime>=1.24"`) and point Drishti at its shared
  library with `ORT_DYLIB_PATH=/path/to/libonnxruntime.so` (or `.dylib` / `.dll`).

---

## Quick start

All three surfaces read the same configuration file, which is where you choose
the model for each check. See [Configuration](#configuration).

**Command line**

```bash
drishti --config config.toml prompt   --text "Ignore all previous instructions."
drishti --config config.toml pii      --text "Email me at jane@example.com"
drishti --config config.toml output   --text "Have a great day!"
drishti --config config.toml all      --text "..." --output "..."
drishti --config config.toml manifest         # loaded model ids and hashes
```

Pass `--file <path>` instead of `--text` to read from a file. Output is
structured JSON.

**HTTP service**

```bash
drishti-server --config config.toml --bind 0.0.0.0:8080 --token <bearer-token>
```

**Python**

```python
import drishti

d = drishti.Drishti.from_config_file("config.toml")
d.check_prompt("Ignore all previous instructions.")
d.check_pii("Email me at jane@example.com")
d.check_output("Have a great day!")
d.manifest()
```

Methods return plain dictionaries and release the interpreter lock during
inference.

---

## The three checks

### Prompt injection

Takes a prompt and returns an injection score from 0.0 to 1.0, a class, a
confidence, the latency, and the model id. It catches common injection patterns
("ignore previous instructions", "you are now DAN", and similar). It is one layer
of defense, not a jailbreak-proof filter.

### PII detection and redaction

Two stages:

- A **regex stage** (always on, about 5 ms) finds structurally identifiable PII:
  emails, credit cards (Luhn validated), phone numbers, IPv4 and IPv6 addresses,
  IBANs, US SSNs, India PAN, Aadhaar and UPI, UK NINO, and EU VAT numbers.
- An optional **NER stage** finds unstructured PII like names, organisations, and
  locations.

The result is a list of spans (byte offsets, kind, confidence, source) plus a
redacted copy of the text. Redaction is chosen per kind: mask, hash, tokenise,
keep, or refuse.

### Output safety

Takes a model output and returns a score per safety category, an aggregate
pass-or-fail against a threshold you set, and the detected language. The taxonomy
comes from the configured model, so any classifier-style safety model fits.

---

## Configuration

Configuration is a TOML file. Every value can also be overridden by an
environment variable or a `.env` file, so tuning never needs a code change or a
rebuild. The override key is `DRISHTI_<PATH>`, with a double underscore between
nesting levels:

```bash
DRISHTI_OUTPUT__THRESHOLD=0.05
DRISHTI_PII__NER__DROP_ACRONYMS=true
DRISHTI_INTRA_THREADS=4
```

A worked example is in [config/example.toml](config/example.toml). A check is
enabled only when its section is present, and an enabled check must name a model
or startup fails with a clear error rather than guessing one.

---

## Models

Drishti hardcodes no model. You choose the model for each check through
configuration: there are no default model ids, URLs, or hashes compiled into the
binary. If a chosen model is already present, Drishti uses it directly. If it is
not, Drishti downloads it once from the configured source, verifies its SHA-256
when you provide one, caches it, and then uses it. To bring your own fine-tuned
model, point the config at a local path.

There is no default model and no bundled weights. Instead Drishti ships a
recommendation matrix: **[MODELS.md](MODELS.md)** lists vetted models per check
across a footprint range (small to large), with precision and recall measured on
public benchmarks, honest notes on where each model fits, and starting points by
industry. Pick a row, point config at it. [config/starter.toml](config/starter.toml)
is a ready-to-run example, one point on that matrix.

A working starter set (used in `config/starter.toml`):

| Check | Model | Weights | Size |
|---|---|---|---|
| Prompt injection | ProtectAI DeBERTa-v3-base prompt-injection-v2 | fp32 | 704 MB |
| PII names and orgs | dslim/distilbert-NER | fp32 | 249 MB |
| Output safety | KoalaAI Text-Moderation | int8 | 136 MB |

> Note: model size is a footprint budget, not a quality ranking. What decides
> accuracy is whether a model was trained on content and labels like yours; see
> [MODELS.md](MODELS.md). Separately, int8 dynamic quantization significantly
> degrades DeBERTa-v3 accuracy, so run that prompt-injection model at full
> precision and switch model family if you need a smaller footprint.

---

## HTTP API

JSON in, JSON out. The check endpoints and the manifest require a bearer token
when one is configured; health and metrics are always open.

| Method | Path | Body | Auth |
|---|---|---|---|
| POST | `/v1/check/prompt` | `{ "input": "..." }` | bearer |
| POST | `/v1/check/pii` | `{ "input": "..." }` | bearer |
| POST | `/v1/check/output` | `{ "output": "..." }` | bearer |
| POST | `/v1/check/all` | `{ "prompt": "...", "output": "..." }` | bearer |
| POST | `/v1/check/prompt/batch` | `{ "inputs": ["...", "..."] }` | bearer |
| POST | `/v1/check/pii/batch` | `{ "inputs": ["...", "..."] }` | bearer |
| POST | `/v1/check/output/batch` | `{ "outputs": ["...", "..."] }` | bearer |
| GET | `/v1/manifest` | loaded model ids and hashes | bearer |
| GET | `/v1/version` | drishti version and model-set id | open |
| GET | `/healthz` | liveness | open |
| GET | `/readyz` | 200 only when models are loaded | open |
| GET | `/metrics` | Prometheus text | open |

Every check body also accepts an optional `"model_set": "<id>"`. When present it
is checked against the loaded set and a mismatch returns HTTP 409; when absent
(the default) it is ignored, so existing callers are unaffected. Running Drishti
as a separate, optionally GPU-backed service that many gateways share, and the
optional TLS, timeout, size, and concurrency limits, are covered in
[SERVING.md](SERVING.md).

```bash
curl -s -X POST http://localhost:8080/v1/check/pii \
  -H "authorization: Bearer <token>" \
  -H "content-type: application/json" \
  -d '{"input": "card 4111 1111 1111 1111"}'
```

Prefer a typed client over raw HTTP? Remote client SDKs for Python
(`sarthiai-drishti-sdk`) and Node (`sarthiai-drishti-sdk`) live in [clients/](clients/),
each with its own README, and a Rust client (`sarthiai-drishti-client`, a
`RemoteDrishti` that implements the same `SafetyEngine` trait as the embedded
engine) lives in [crates/drishti-client/](crates/drishti-client/). They call a
running `drishti-server` and return typed results; they load no model themselves.
This is distinct from the in-process Python package (`import drishti`) shown
above, which runs the models locally.

---

## Eval results

These figures come from the eval harness ([eval/](eval/)) run through the real
engine on recognized public benchmarks: `deepset/prompt-injections`, the OpenAI
moderation evaluation, and an `ai4privacy/pii-masking-200k` English sample. They
measure the specific models configured, not Drishti in the abstract: accuracy is
a property of the model you pick. The full per-model, per-tier matrix and how to
choose is in [MODELS.md](MODELS.md). Reproduce with `cargo run -p drishti-eval --
--config <cfg> --datasets <dir>`; the report, including the SHA-256 of every model
used, is written under `eval/results/`.

Measured highlights (validated means it cleared its bar):

| Check (model) | Precision | Recall | F1 | Verdict |
|---|---|---|---|---|
| Output safety (KoalaAI Text-Moderation) | 0.879 | 0.960 | 0.918 | validated |
| PII regex, Email | 0.996 | 0.939 | 0.967 | validated |
| PII regex, IBAN | 1.000 | 1.000 | 1.000 | validated |
| PII NER, PersonName (distilbert to bert-large) | up to 0.840 | up to 0.919 | 0.86 to 0.88 | experimental |
| Prompt injection (ProtectAI DeBERTa) | 0.965 | 0.414 | 0.580 | experimental |

> The prompt-injection recall is low because this benchmark is multilingual and
> out of distribution for that English model; a different model scores very
> differently. That is the point of [MODELS.md](MODELS.md): public numbers do not
> predict your traffic. Every runtime result stays labelled experimental until its
> configured path clears its bar on a real benchmark.

---

## Performance

Warm inference on commodity CPU hardware: the regex PII stage runs in about 5 ms;
the NER and output-safety classifiers in tens of milliseconds; the
prompt-injection model at full precision is the heaviest, in the low hundreds of
milliseconds. A cold process additionally pays a one-time model load, which the
persistent server amortizes. Detailed p50 and p99 figures are published each
release.

---

## Threat model

**In scope:** naive prompt injection (instruction-override patterns), common PII
in inputs and outputs (emails, cards, phones, names, addresses, and the
structured identifiers above), common harmful output content in English, and
tampering with model files (caught by SHA-256 verification when a hash is set).

**Out of scope:** adversarial prompts crafted to evade a specific classifier,
jailbreaks that do not use injection patterns (roleplay, hypothetical framing),
non-English content (Drishti reports the detected language and lowers its
confidence), PII obfuscated through unusual encodings, and attacks on the host
process itself. Drishti is one layer of defense: it reports scores, and
enforcement belongs to your policy layer.

---

## Project layout

```
drishti/
  crates/
    drishti-core/         detection logic and public types
    drishti-models/       model resolution, download, caching, hash verification
    drishti-regex/        the PII regex recognizer set
    drishti-server/       the axum HTTP service
    drishti-ffi-python/   the PyO3 bindings (in-process Python package)
    drishti-cli/          the command-line tool
  clients/
    python/               drishti-sdk: remote HTTP client for Python
    node/                 @sarthiai/drishti-sdk: remote HTTP client for Node
  eval/                   the eval harness, datasets, benchmarks, and results
  config/                 example configuration
  MODELS.md               model recommendation matrix (no default model)
```

---

## License

Drishti is licensed under the **Elastic License 2.0 (ELv2)**. Licensor: Chirotpal
Das. See [LICENSE](LICENSE) for the full text. In short: you may use, copy,
modify, and distribute it, but you may not offer it to third parties as a hosted
or managed service, and you may not remove the licensing notices.

---

## Part of Niyam

Drishti is the content-safety piece of the Niyam family: Kavach (armor) protects,
Drishti (sight) watches, Lipi (script) writes the rules. Drishti is useful on its
own and integrates with the rest through Niyam's shared contracts in later
versions. The decision layer (what to block or allow) is Kavach, with rules
authored in Lipi. Drishti only ever hands over scores and flags.

---

<div align="center">

Designed, developed, and maintained by <a href="https://www.linkedin.com/in/chirotpal/" target="_blank">Chirotpal</a>

</div>
