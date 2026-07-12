"""Export one Hugging Face model to ONNX, benchmark it THROUGH the real Drishti
engine (drishti-eval), record the numbers, then delete the model to save disk.

Usage:
  bench_model.py --hf-id <id> --check {prompt,ner,output} --tier <name> \
      --eval-bin <path> --sets-root <dir> --results-dir <dir> --work <dir>

Config (labels, positive_label, categories, safe_category) is derived from the
model's own config.json id2label, so nothing is hardcoded per model.
"""

import argparse
import json
import shutil
import subprocess
import sys
from pathlib import Path

INJECTION_HINTS = ("inject", "jailbreak", "malicious", "unsafe", "attack", "prompt")
SAFE_HINTS = ("ok", "safe", "neutral", "clean", "none", "non-toxic", "notoxic", "not_toxic")


def ordered_labels(config: dict) -> list:
    id2label = config.get("id2label") or {}
    if not id2label:
        return []
    return [id2label[str(i)] for i in range(len(id2label))]


def export(hf_id: str, task: str, out: Path, cli: str) -> None:
    out.mkdir(parents=True, exist_ok=True)
    cmd = [cli, "export", "onnx", "--model", hf_id, "--task", task, str(out)]
    subprocess.run(cmd, check=True)


def build_config(check: str, model_dir: Path, hf_id: str) -> str:
    cfg = json.loads((model_dir / "config.json").read_text())
    labels = ordered_labels(cfg)
    onnx = model_dir / "model.onnx"
    tok = model_dir / "tokenizer.json"
    common = f'''intra_threads = 4
'''
    if check == "prompt":
        # index of the injection logit
        pos = 1
        for i, lab in enumerate(labels):
            if any(h in lab.lower() for h in INJECTION_HINTS):
                pos = i
                break
        return common + f'''
[prompt]
max_tokens = 512
positive_label = {pos}

  [prompt.model]
  id = "{hf_id}"
    [prompt.model.model]
    source = "local"
    location = "{onnx}"
    [prompt.model.tokenizer]
    source = "local"
    location = "{tok}"
'''
    if check == "ner":
        labels_toml = ", ".join(f'"{l}"' for l in labels)
        return common + f'''
[pii]
regex_enabled = true

  [pii.ner]
  labels = [{labels_toml}]
  max_tokens = 512
  threshold = 0.5
  drop_acronyms = true
  drop_types = ["MISC"]

    [pii.ner.model]
    id = "{hf_id}"
      [pii.ner.model.model]
      source = "local"
      location = "{onnx}"
      [pii.ner.model.tokenizer]
      source = "local"
      location = "{tok}"

  [pii.redaction]
  default = "mask"
'''
    if check == "output":
        cats_toml = ", ".join(f'"{l}"' for l in labels)
        safe = next((l for l in labels if l.lower() in SAFE_HINTS), None)
        if safe:
            multi = "false"
            safe_line = f'safe_category = "{safe}"'
            threshold = 0.1
        else:
            multi = "true"
            safe_line = ""
            threshold = 0.5
        return common + f'''
[output]
categories = [{cats_toml}]
multi_label = {multi}
{safe_line}
threshold = {threshold}
max_tokens = 512

  [output.model]
  id = "{hf_id}"
    [output.model.model]
    source = "local"
    location = "{onnx}"
    [output.model.tokenizer]
    source = "local"
    location = "{tok}"
'''
    raise ValueError(check)


TASK = {"prompt": "text-classification", "ner": "token-classification", "output": "text-classification"}
SET_DIR = {"prompt": "prompt", "ner": "pii", "output": "output"}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--hf-id", required=True)
    ap.add_argument("--check", required=True, choices=["prompt", "ner", "output"])
    ap.add_argument("--tier", required=True)
    ap.add_argument("--eval-bin", required=True)
    ap.add_argument("--sets-root", required=True)
    ap.add_argument("--results-dir", required=True)
    ap.add_argument("--work", required=True)
    ap.add_argument("--optimum-cli", required=True)
    args = ap.parse_args()

    work = Path(args.work)
    model_dir = work / "model"
    if model_dir.exists():
        shutil.rmtree(model_dir)

    print(f"\n===== {args.check} / {args.tier} / {args.hf_id} =====", flush=True)
    try:
        export(args.hf_id, TASK[args.check], model_dir, args.optimum_cli)
    except subprocess.CalledProcessError:
        print(f"EXPORT FAILED for {args.hf_id}", flush=True)
        return 2

    cfg_text = build_config(args.check, model_dir, args.hf_id)
    cfg_path = work / "config.toml"
    cfg_path.write_text(cfg_text)

    sets_dir = Path(args.sets_root) / SET_DIR[args.check]
    result_path = Path(args.results_dir) / f"{args.check}-{args.tier}.json"
    result_path.parent.mkdir(parents=True, exist_ok=True)

    try:
        subprocess.run(
            [args.eval_bin, "--config", str(cfg_path), "--datasets", str(sets_dir),
             "--out", str(result_path), "--label", f"{args.tier}:{args.hf_id}"],
            check=True,
        )
    except subprocess.CalledProcessError:
        print(f"EVAL FAILED for {args.hf_id}", flush=True)
        shutil.rmtree(model_dir, ignore_errors=True)
        return 3

    # Free disk immediately.
    shutil.rmtree(model_dir, ignore_errors=True)
    print(f"saved {result_path}, model deleted", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
