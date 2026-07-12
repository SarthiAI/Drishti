"""Typed result models mirroring drishti-core/src/types.rs.

Every result carries its `validation` status (`validated` or `experimental`) so
a caller never mistakes an unvalidated path for a contractual one.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Dict, List, Optional, Union


def _normalize_kind(raw: Union[str, Dict[str, Any]]) -> str:
    """PiiKind serializes as a bare string for named variants and as
    {"Other": "tag"} for unknown tags. Normalize both to a label string."""
    if isinstance(raw, dict):
        # Externally tagged enum variant, e.g. {"Other": "SomeTag"}.
        for value in raw.values():
            return str(value)
        return "Other"
    return str(raw)


@dataclass(frozen=True)
class PromptCheck:
    score: float
    class_: str
    confidence: float
    latency_ms: int
    model_id: str
    truncated: bool
    validation: str

    @classmethod
    def from_dict(cls, d: Dict[str, Any]) -> "PromptCheck":
        return cls(
            score=float(d["score"]),
            class_=str(d["class"]),
            confidence=float(d["confidence"]),
            latency_ms=int(d["latency_ms"]),
            model_id=str(d["model_id"]),
            truncated=bool(d["truncated"]),
            validation=str(d["validation"]),
        )


@dataclass(frozen=True)
class PiiSpan:
    start: int
    end: int
    kind: str
    confidence: float
    source: str  # "regex" or "ner"

    @classmethod
    def from_dict(cls, d: Dict[str, Any]) -> "PiiSpan":
        return cls(
            start=int(d["start"]),
            end=int(d["end"]),
            kind=_normalize_kind(d["kind"]),
            confidence=float(d["confidence"]),
            source=str(d["source"]),
        )


@dataclass(frozen=True)
class PiiCheck:
    spans: List[PiiSpan]
    redacted: str
    refuse: bool
    latency_ms: int
    regex_version: str
    ner_model_id: Optional[str]
    validation: str

    @classmethod
    def from_dict(cls, d: Dict[str, Any]) -> "PiiCheck":
        return cls(
            spans=[PiiSpan.from_dict(s) for s in d.get("spans", [])],
            redacted=str(d["redacted"]),
            refuse=bool(d["refuse"]),
            latency_ms=int(d["latency_ms"]),
            regex_version=str(d["regex_version"]),
            ner_model_id=(d["ner_model_id"] if d.get("ner_model_id") is not None else None),
            validation=str(d["validation"]),
        )


@dataclass(frozen=True)
class OutputCheck:
    categories: Dict[str, float]
    overall: str  # "pass", "fail", or "uncertain"
    language: str
    latency_ms: int
    model_id: str
    validation: str

    @classmethod
    def from_dict(cls, d: Dict[str, Any]) -> "OutputCheck":
        return cls(
            categories={str(k): float(v) for k, v in d.get("categories", {}).items()},
            overall=str(d["overall"]),
            language=str(d["language"]),
            latency_ms=int(d["latency_ms"]),
            model_id=str(d["model_id"]),
            validation=str(d["validation"]),
        )


@dataclass(frozen=True)
class FullCheck:
    prompt: Optional[PromptCheck]
    pii: Optional[PiiCheck]
    output: Optional[OutputCheck]

    @classmethod
    def from_dict(cls, d: Dict[str, Any]) -> "FullCheck":
        return cls(
            prompt=PromptCheck.from_dict(d["prompt"]) if d.get("prompt") else None,
            pii=PiiCheck.from_dict(d["pii"]) if d.get("pii") else None,
            output=OutputCheck.from_dict(d["output"]) if d.get("output") else None,
        )


@dataclass(frozen=True)
class ModelManifestEntry:
    role: str
    model_id: str
    sha256: str

    @classmethod
    def from_dict(cls, d: Dict[str, Any]) -> "ModelManifestEntry":
        return cls(role=str(d["role"]), model_id=str(d["model_id"]), sha256=str(d["sha256"]))


@dataclass(frozen=True)
class ModelManifest:
    regex_version: str
    models: List[ModelManifestEntry]

    @classmethod
    def from_dict(cls, d: Dict[str, Any]) -> "ModelManifest":
        return cls(
            regex_version=str(d["regex_version"]),
            models=[ModelManifestEntry.from_dict(m) for m in d.get("models", [])],
        )
