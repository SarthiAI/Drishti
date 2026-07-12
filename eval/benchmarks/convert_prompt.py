"""Convert deepset/prompt-injections into the drishti-eval prompt format.

Output line shape: {"text": "...", "label": "injection" | "benign"}
The dataset labels 1 = injection, 0 = legitimate. Both splits are used since we
are evaluating a third-party model, not honoring a train/test boundary.
"""

import json
import sys
from pathlib import Path

from datasets import load_dataset

OUT = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("datasets-full/prompt_injection.jsonl")


def main() -> None:
    ds = load_dataset("deepset/prompt-injections")
    rows = []
    for split in ds:
        for r in ds[split]:
            text = (r.get("text") or "").strip()
            if not text:
                continue
            label = int(r["label"])
            rows.append({"text": text, "label": "injection" if label == 1 else "benign"})
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text("\n".join(json.dumps(r, ensure_ascii=False) for r in rows))
    inj = sum(1 for r in rows if r["label"] == "injection")
    print(f"wrote {len(rows)} prompt examples to {OUT} ({inj} injection, {len(rows) - inj} benign)")


if __name__ == "__main__":
    main()
