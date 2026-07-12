//! The public result types. Every check returns a score and the identity of the
//! model that produced it. No check returns a policy decision. See invariant I1.

use std::collections::HashMap;

use serde::Serialize;

/// Whether the path that produced a result has cleared its eval-harness bar.
/// Carried in every result so a caller never mistakes an unvalidated path for a
/// contractual one. See invariant I7.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Validation {
    Validated,
    Experimental,
}

/// Prompt-injection sub-class. v0.1 emits `Benign` or `InstructionOverride`; the
/// finer classes are reserved and experimental until labelled data validates them.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub enum PromptClass {
    Benign,
    InstructionOverride,
    GoalHijack,
    DataExfil,
    ToolMisuse,
}

#[derive(Clone, Debug, Serialize)]
pub struct PromptCheck {
    pub score: f32,
    pub class: PromptClass,
    pub confidence: f32,
    pub latency_ms: u32,
    pub model_id: String,
    pub truncated: bool,
    pub validation: Validation,
}

/// Where a PII span came from.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PiiSource {
    Regex,
    Ner,
}

/// PII kind. Not exhaustive: operators add custom recognizers, and anything the
/// recognizer set does not name maps to `Other`.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, Hash)]
pub enum PiiKind {
    Email,
    Phone,
    CreditCard,
    Iban,
    IpAddress,
    Ssn,
    Pan,
    Aadhaar,
    Upi,
    Passport,
    DriversLicense,
    PersonName,
    Address,
    Organisation,
    Location,
    DateOfBirth,
    MedicalRecord,
    Nino,
    Vat,
    Other(String),
}

impl PiiKind {
    /// Map a recognizer or NER label tag onto a kind. Unknown tags become
    /// `Other`, so a configured NER label set never fails to map.
    pub fn from_tag(tag: &str) -> Self {
        match tag {
            "Email" => Self::Email,
            "Phone" => Self::Phone,
            "CreditCard" => Self::CreditCard,
            "IBAN" => Self::Iban,
            "IPAddress" => Self::IpAddress,
            "SSN" => Self::Ssn,
            "PAN" => Self::Pan,
            "Aadhaar" => Self::Aadhaar,
            "UPI" => Self::Upi,
            "Passport" => Self::Passport,
            "DriversLicense" => Self::DriversLicense,
            "NINO" => Self::Nino,
            "VAT" => Self::Vat,
            // Common NER entity-type names.
            "PER" | "PERSON" | "PersonName" => Self::PersonName,
            "LOC" | "LOCATION" | "Location" => Self::Location,
            "ORG" | "ORGANISATION" | "ORGANIZATION" | "Organisation" => Self::Organisation,
            "ADDRESS" | "Address" => Self::Address,
            "DOB" | "DateOfBirth" => Self::DateOfBirth,
            "MEDICAL" | "MedicalRecord" => Self::MedicalRecord,
            other => Self::Other(other.to_string()),
        }
    }

    /// A short stable label, used for redaction markers and per-kind policy keys.
    pub fn label(&self) -> String {
        match self {
            Self::Other(s) => s.clone(),
            other => format!("{other:?}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct PiiSpan {
    /// Byte offset into the input, inclusive start.
    pub start: usize,
    /// Byte offset into the input, exclusive end.
    pub end: usize,
    pub kind: PiiKind,
    pub confidence: f32,
    pub source: PiiSource,
}

#[derive(Clone, Debug, Serialize)]
pub struct PiiCheck {
    pub spans: Vec<PiiSpan>,
    pub redacted: String,
    /// True when at least one span had the `Refuse` strategy, so the caller can
    /// refuse the whole request. This is a flag for the caller, not a decision.
    pub refuse: bool,
    pub latency_ms: u32,
    pub regex_version: String,
    pub ner_model_id: Option<String>,
    pub validation: Validation,
}

/// Aggregate pass/fail for output safety, against the configured threshold.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SafetyVerdict {
    Pass,
    Fail,
    Uncertain,
}

#[derive(Clone, Debug, Serialize)]
pub struct OutputCheck {
    pub categories: HashMap<String, f32>,
    pub overall: SafetyVerdict,
    /// Best-effort ISO 639-1 language code, or "und" when undetermined.
    pub language: String,
    pub latency_ms: u32,
    pub model_id: String,
    pub validation: Validation,
}

/// Result of `check_all`. Each field is present only if that check ran.
#[derive(Clone, Debug, Serialize, Default)]
pub struct FullCheck {
    pub prompt: Option<PromptCheck>,
    pub pii: Option<PiiCheck>,
    pub output: Option<OutputCheck>,
}

/// One entry per loaded model, for audit. Reports exactly which artifact
/// produced results, regardless of how it was sourced. See invariant I6.
#[derive(Clone, Debug, Serialize)]
pub struct ModelManifestEntry {
    pub role: String,
    pub model_id: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct ModelManifest {
    pub regex_version: String,
    pub models: Vec<ModelManifestEntry>,
}
