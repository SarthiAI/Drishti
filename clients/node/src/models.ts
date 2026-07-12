// Typed result models mirroring drishti-core/src/types.rs.
// Every result carries its `validation` status ("validated" | "experimental")
// so a caller never mistakes an unvalidated path for a contractual one.

export type Validation = "validated" | "experimental";

export interface PromptCheck {
  score: number;
  class: string;
  confidence: number;
  latencyMs: number;
  modelId: string;
  truncated: boolean;
  validation: Validation;
}

export interface PiiSpan {
  start: number;
  end: number;
  kind: string;
  confidence: number;
  source: "regex" | "ner";
}

export interface PiiCheck {
  spans: PiiSpan[];
  redacted: string;
  refuse: boolean;
  latencyMs: number;
  regexVersion: string;
  nerModelId: string | null;
  validation: Validation;
}

export interface OutputCheck {
  categories: Record<string, number>;
  overall: "pass" | "fail" | "uncertain";
  language: string;
  latencyMs: number;
  modelId: string;
  validation: Validation;
}

export interface FullCheck {
  prompt: PromptCheck | null;
  pii: PiiCheck | null;
  output: OutputCheck | null;
}

export interface ModelManifestEntry {
  role: string;
  modelId: string;
  sha256: string;
}

export interface ModelManifest {
  regexVersion: string;
  models: ModelManifestEntry[];
}

// ---- parsers: server JSON (snake_case) into typed models -----------------

function normalizeKind(raw: unknown): string {
  // PiiKind serializes as a bare string for named variants and as
  // { "Other": "tag" } for unknown tags. Normalize both to a label string.
  if (raw && typeof raw === "object") {
    const values = Object.values(raw as Record<string, unknown>);
    return values.length > 0 ? String(values[0]) : "Other";
  }
  return String(raw);
}

export function parsePromptCheck(d: any): PromptCheck {
  return {
    score: Number(d.score),
    class: String(d.class),
    confidence: Number(d.confidence),
    latencyMs: Number(d.latency_ms),
    modelId: String(d.model_id),
    truncated: Boolean(d.truncated),
    validation: d.validation as Validation,
  };
}

export function parsePiiSpan(d: any): PiiSpan {
  return {
    start: Number(d.start),
    end: Number(d.end),
    kind: normalizeKind(d.kind),
    confidence: Number(d.confidence),
    source: d.source,
  };
}

export function parsePiiCheck(d: any): PiiCheck {
  return {
    spans: Array.isArray(d.spans) ? d.spans.map(parsePiiSpan) : [],
    redacted: String(d.redacted),
    refuse: Boolean(d.refuse),
    latencyMs: Number(d.latency_ms),
    regexVersion: String(d.regex_version),
    nerModelId: d.ner_model_id ?? null,
    validation: d.validation as Validation,
  };
}

export function parseOutputCheck(d: any): OutputCheck {
  const categories: Record<string, number> = {};
  for (const [k, v] of Object.entries(d.categories ?? {})) {
    categories[k] = Number(v);
  }
  return {
    categories,
    overall: d.overall,
    language: String(d.language),
    latencyMs: Number(d.latency_ms),
    modelId: String(d.model_id),
    validation: d.validation as Validation,
  };
}

export function parseFullCheck(d: any): FullCheck {
  return {
    prompt: d.prompt ? parsePromptCheck(d.prompt) : null,
    pii: d.pii ? parsePiiCheck(d.pii) : null,
    output: d.output ? parseOutputCheck(d.output) : null,
  };
}

export function parseModelManifest(d: any): ModelManifest {
  return {
    regexVersion: String(d.regex_version),
    models: Array.isArray(d.models)
      ? d.models.map((m: any) => ({
          role: String(m.role),
          modelId: String(m.model_id),
          sha256: String(m.sha256),
        }))
      : [],
  };
}
