"""Convert the OpenAI moderation evaluation set into the drishti-eval output format.

Output line shape: {"text": "...", "label": "unsafe" | "safe"}
A record is unsafe if any moderation category flag is set, safe otherwise. The
category columns are auto-detected (every non-text column that holds a 0/1 flag),
so this works regardless of the exact category naming.
"""

import json
import sys
from pathlib import Path

from datasets import load_dataset

OUT = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("datasets-full/output_safety.jsonl")
DATASET = "mmathys/openai-moderation-api-evaluation"


def truthy(v) -> bool:
    if v is None:
        return False
    if isinstance(v, bool):
        return v
    if isinstance(v, (int, float)):
        return v != 0
    if isinstance(v, str):
        return v.strip().lower() not in ("", "0", "false", "no")
    return bool(v)


def main() -> None:
    ds = load_dataset(DATASET)
    rows = []
    for split in ds:
        first = ds[split][0]
        text_col = "prompt" if "prompt" in first else ("text" if "text" in first else list(first)[0])
        cat_cols = [k for k in first.keys() if k != text_col]
        for r in ds[split]:
            text = (r.get(text_col) or "").strip()
            if not text:
                continue
            unsafe = any(truthy(r.get(c)) for c in cat_cols)
            rows.append({"text": text, "label": "unsafe" if unsafe else "safe"})
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text("\n".join(json.dumps(r, ensure_ascii=False) for r in rows))
    unsafe = sum(1 for r in rows if r["label"] == "unsafe")
    print(f"wrote {len(rows)} output examples to {OUT} ({unsafe} unsafe, {len(rows) - unsafe} safe)")
    print(f"category columns detected: {cat_cols}")


if __name__ == "__main__":
    main()
