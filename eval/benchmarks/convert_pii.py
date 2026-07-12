"""Convert ai4privacy/pii-masking-200k (English) into the drishti-eval PII format.

Output line shape: {"text": "...", "kinds": ["Email", "PersonName", ...]}

ai4privacy has ~50 fine-grained labels; we map only the ones Drishti actually
detects (regex kinds plus NER person/location/organisation) and DROP the rest
from the truth set, so the benchmark measures Drishti on the kinds it claims,
not on kinds it never emits. A fixed English prefix of the data is used so the
run is reproducible. Sample size is the first CLI arg (default 3000).
"""

import json
import sys
from pathlib import Path

from datasets import load_dataset

OUT = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("datasets-full/pii.jsonl")
LIMIT = int(sys.argv[2]) if len(sys.argv) > 2 else 3000

# ai4privacy label -> Drishti kind. Labels not listed here are dropped from the
# truth set (Drishti does not claim to detect them).
LABEL_MAP = {
    "EMAIL": "Email",
    "PHONENUMBER": "Phone",
    "CREDITCARDNUMBER": "CreditCard",
    "IBAN": "Iban",
    "IP": "IpAddress",
    "IPV4": "IpAddress",
    "IPV6": "IpAddress",
    "SSN": "Ssn",
    "FIRSTNAME": "PersonName",
    "LASTNAME": "PersonName",
    "MIDDLENAME": "PersonName",
    "CITY": "Location",
    "STATE": "Location",
    "COUNTY": "Location",
    "STREET": "Location",
    "STREETADDRESS": "Location",
    "SECONDARYADDRESS": "Location",
    "COMPANYNAME": "Organisation",
}


def main() -> None:
    ds = load_dataset("ai4privacy/pii-masking-200k", split="train")
    rows = []
    for r in ds:
        if r.get("language") != "en":
            continue
        text = (r.get("source_text") or "").strip()
        if not text:
            continue
        kinds = set()
        for span in r.get("privacy_mask") or []:
            mapped = LABEL_MAP.get(str(span.get("label", "")).upper())
            if mapped:
                kinds.add(mapped)
        rows.append({"text": text, "kinds": sorted(kinds)})
        if len(rows) >= LIMIT:
            break

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text("\n".join(json.dumps(r, ensure_ascii=False) for r in rows))

    from collections import Counter
    c = Counter(k for r in rows for k in r["kinds"])
    with_pii = sum(1 for r in rows if r["kinds"])
    print(f"wrote {len(rows)} pii examples to {OUT} ({with_pii} with mapped PII, {len(rows) - with_pii} without)")
    print("per-kind truth counts:", dict(sorted(c.items(), key=lambda x: -x[1])))


if __name__ == "__main__":
    main()
