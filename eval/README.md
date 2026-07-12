# drishti-eval

The validation gate. It runs labelled eval sets through a real Drishti instance,
computes precision / recall / F1, applies the validated-vs-experimental bars, and
writes a reproducible JSON report with the exact model hashes that produced it.

## Run it

```
cargo run -p drishti-eval -- --config <your-config.toml>
# options:
#   --datasets eval/datasets        directory of .jsonl datasets
#   --out      eval/results/latest.json
```

It evaluates whichever datasets are present, so you can run a single check by
keeping only its dataset file.

## Datasets

Curated seed sets, not the full public benchmarks. They are sized to exercise the
harness and surface the validated-versus-experimental split; the full benchmarks
are wired in for release numbers. Format is JSON-lines:

- `prompt_injection.jsonl`: `{"text": "...", "label": "injection" | "benign"}`
- `pii.jsonl`: `{"text": "...", "kinds": ["Email", "PersonName", ...]}` where
  `kinds` lists every PII kind truly present, using Drishti's kind labels. Empty
  means no PII. Matching is presence-based per kind (did we detect kind K in a
  text whose truth set contains K).
- `output_safety.jsonl`: `{"text": "...", "label": "unsafe" | "safe"}`. Evaluated
  as a binary safe/unsafe verdict.

## Validation bars

- Prompt injection: F1 >= 0.92.
- PII: precision >= 0.90 and recall >= 0.85, per kind.
- Output safety: F1 >= 0.85 (binary safe/unsafe here; per-category F1 needs
  category-labelled data, a follow-up).

## What "validated" means here

The harness reports a `seed_verdict` per path: did it clear the bar on THIS seed
set. That is not the same as the runtime label. A path is considered validated
only once it clears its bar on the full public benchmarks and the cross-surface
consumer harness. Until then every runtime result stays `experimental`. This
harness produces the numbers; it does not flip that label.

## Reproducibility

`eval/results/latest.json` records the metrics alongside the `model_id` and
`sha256` of every model used, so any number can be traced to the exact artifact
that produced it. Re-running with the same models and datasets reproduces it.

## Roadmap

- Wire the full public benchmarks (PINT, InjecGuard, Presidio eval, OpenAI
  Moderation eval).
- Span-level PII matching (currently presence-based per kind).
- Per-category output-safety F1 (needs category-labelled data).
