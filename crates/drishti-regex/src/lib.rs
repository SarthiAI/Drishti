//! The versioned PII regex recognizer set.
//!
//! This crate is pure: it depends only on `regex`, performs no I/O, and knows
//! nothing about `drishti-core`. It reports structurally identifiable PII with
//! byte offsets so the caller can build spans and apply redaction. Unstructured
//! PII (names, addresses in prose) is the NER stage's job, in core.
//!
//! Every match carries a `kind` tag (a stable string) and a confidence. The
//! caller maps the tag onto its own `PiiKind`. The recognizer set is versioned
//! via [`REGEX_VERSION`] so results can cite exactly which rules produced them.

use std::sync::LazyLock;

use regex::Regex;

/// Version of the recognizer set. Bump on any rule change so results stay
/// traceable to the exact rules that produced them.
pub const REGEX_VERSION: &str = "regex-2026.06.08";

/// A single structurally-identified PII match, in byte offsets over the input.
#[derive(Clone, Debug, PartialEq)]
pub struct RegexPiiMatch {
    pub start: usize,
    pub end: usize,
    /// Stable kind tag, mapped to a richer enum by the caller.
    pub kind: &'static str,
    /// Calibrated-ish confidence for this rule. Structural rules are high.
    pub confidence: f32,
}

struct Recognizer {
    kind: &'static str,
    re: Regex,
    confidence: f32,
    /// Optional extra validation on the matched text (e.g. Luhn). When present
    /// and it returns false, the match is dropped.
    validate: Option<fn(&str) -> bool>,
}

// Each pattern is anchored on word-ish boundaries where reasonable. We keep the
// rules conservative: a false negative is recoverable by the NER stage or a
// custom recognizer, a false positive corrupts a redaction.
static RECOGNIZERS: LazyLock<Vec<Recognizer>> = LazyLock::new(|| {
    vec![
        Recognizer {
            kind: "Email",
            re: Regex::new(r"(?i)\b[a-z0-9._%+\-]+@[a-z0-9.\-]+\.[a-z]{2,}\b").unwrap(),
            confidence: 0.97,
            validate: None,
        },
        // IPv4, with each octet in 0..=255 enforced by the validator.
        Recognizer {
            kind: "IPAddress",
            re: Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b").unwrap(),
            confidence: 0.9,
            validate: Some(valid_ipv4),
        },
        // IPv6, loose form. Validator keeps it honest.
        Recognizer {
            kind: "IPAddress",
            re: Regex::new(r"\b(?:[A-Fa-f0-9]{1,4}:){2,7}[A-Fa-f0-9]{1,4}\b").unwrap(),
            confidence: 0.85,
            validate: Some(valid_ipv6_loose),
        },
        // Candidate card numbers: 13 to 19 digits, optionally grouped by space
        // or dash. Anchored to start and end on a digit so a trailing separator
        // is never absorbed into the span. Luhn drops the non-cards.
        Recognizer {
            kind: "CreditCard",
            re: Regex::new(r"\b\d(?:[ \-]?\d){12,18}\b").unwrap(),
            confidence: 0.95,
            validate: Some(valid_luhn),
        },
        Recognizer {
            kind: "IBAN",
            re: Regex::new(r"\b[A-Z]{2}\d{2}[A-Z0-9]{11,30}\b").unwrap(),
            confidence: 0.93,
            validate: Some(valid_iban),
        },
        // E.164 and common national phone shapes. Kept narrow to limit noise.
        Recognizer {
            kind: "Phone",
            re: Regex::new(r"(?:\+\d{1,3}[ \-]?)?(?:\(\d{1,4}\)[ \-]?)?\d{2,4}[ \-]?\d{3,4}[ \-]?\d{3,4}")
                .unwrap(),
            confidence: 0.75,
            validate: Some(valid_phone),
        },
        // US SSN, dashed or spaced. Excludes obvious invalids via validator.
        Recognizer {
            kind: "SSN",
            re: Regex::new(r"\b\d{3}[ \-]\d{2}[ \-]\d{4}\b").unwrap(),
            confidence: 0.9,
            validate: Some(valid_us_ssn),
        },
        // India PAN: five letters, four digits, one letter.
        Recognizer {
            kind: "PAN",
            re: Regex::new(r"\b[A-Z]{5}\d{4}[A-Z]\b").unwrap(),
            confidence: 0.95,
            validate: None,
        },
        // India Aadhaar: 12 digits, often grouped 4-4-4. First digit 2..=9.
        Recognizer {
            kind: "Aadhaar",
            re: Regex::new(r"\b[2-9]\d{3}[ \-]?\d{4}[ \-]?\d{4}\b").unwrap(),
            confidence: 0.85,
            validate: Some(valid_aadhaar),
        },
        // India UPI virtual payment address: handle@psp.
        Recognizer {
            kind: "UPI",
            re: Regex::new(r"(?i)\b[a-z0-9.\-_]{2,256}@[a-z]{2,64}\b").unwrap(),
            confidence: 0.7,
            validate: Some(valid_upi),
        },
        // UK National Insurance Number.
        Recognizer {
            kind: "NINO",
            re: Regex::new(r"(?i)\b[ABCEGHJ-PRSTW-Z][ABEGHJ-NPRSTW-Z] ?\d{2} ?\d{2} ?\d{2} ?[A-D]\b")
                .unwrap(),
            confidence: 0.9,
            validate: None,
        },
        // EU VAT number: two-letter country code then 2 to 12 alphanumerics.
        Recognizer {
            kind: "VAT",
            re: Regex::new(r"\b(?:AT|BE|BG|CY|CZ|DE|DK|EE|EL|ES|FI|FR|HR|HU|IE|IT|LT|LU|LV|MT|NL|PL|PT|RO|SE|SI|SK)[A-Z0-9]{2,12}\b")
                .unwrap(),
            confidence: 0.8,
            validate: None,
        },
    ]
});

/// Scan text and return every structural PII match, sorted by start offset.
/// Overlapping matches from different recognizers are all returned; the caller
/// resolves overlaps when applying redaction.
pub fn scan(text: &str) -> Vec<RegexPiiMatch> {
    let mut out = Vec::new();
    for rec in RECOGNIZERS.iter() {
        for m in rec.re.find_iter(text) {
            let matched = m.as_str();
            if let Some(v) = rec.validate {
                if !v(matched) {
                    continue;
                }
            }
            out.push(RegexPiiMatch {
                start: m.start(),
                end: m.end(),
                kind: rec.kind,
                confidence: rec.confidence,
            });
        }
    }
    out.sort_by_key(|m| (m.start, m.end));
    out
}

// --- validators ---

fn digits_only(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_digit()).collect()
}

fn valid_luhn(s: &str) -> bool {
    let digits = digits_only(s);
    if digits.len() < 13 || digits.len() > 19 {
        return false;
    }
    let mut sum = 0u32;
    let mut alt = false;
    for c in digits.chars().rev() {
        let mut d = c.to_digit(10).unwrap();
        if alt {
            d *= 2;
            if d > 9 {
                d -= 9;
            }
        }
        sum += d;
        alt = !alt;
    }
    sum % 10 == 0
}

fn valid_ipv4(s: &str) -> bool {
    s.split('.').all(|o| o.parse::<u16>().map(|n| n <= 255).unwrap_or(false))
        && s.split('.').count() == 4
}

fn valid_ipv6_loose(s: &str) -> bool {
    // The regex already constrains shape; require at least two colons and that
    // every group parses as hex. This rejects times like 12:30:45.
    let groups: Vec<&str> = s.split(':').collect();
    groups.len() >= 3 && groups.iter().all(|g| !g.is_empty() && u32::from_str_radix(g, 16).is_ok())
}

fn valid_iban(s: &str) -> bool {
    // ISO 7064 mod-97-10. Move the first four chars to the end, map letters to
    // numbers (A=10..Z=35), and require the big number mod 97 == 1.
    let cleaned: String = s.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if cleaned.len() < 15 || cleaned.len() > 34 {
        return false;
    }
    let (head, tail) = cleaned.split_at(4);
    let rearranged = format!("{tail}{head}");
    let mut remainder = 0u32;
    for c in rearranged.chars() {
        let value = if c.is_ascii_digit() {
            c.to_digit(10).unwrap()
        } else {
            (c.to_ascii_uppercase() as u32) - ('A' as u32) + 10
        };
        // Fold digit by digit to avoid overflow on long IBANs.
        for d in value.to_string().chars() {
            remainder = (remainder * 10 + d.to_digit(10).unwrap()) % 97;
        }
    }
    remainder == 1
}

fn valid_phone(s: &str) -> bool {
    let d = digits_only(s);
    (7..=15).contains(&d.len())
}

fn valid_us_ssn(s: &str) -> bool {
    let d = digits_only(s);
    if d.len() != 9 {
        return false;
    }
    let area = &d[0..3];
    let group = &d[3..5];
    let serial = &d[5..9];
    area != "000" && area != "666" && !area.starts_with('9') && group != "00" && serial != "0000"
}

fn valid_aadhaar(s: &str) -> bool {
    let d = digits_only(s);
    d.len() == 12 && !d.starts_with('0') && !d.starts_with('1')
}

fn valid_upi(s: &str) -> bool {
    // A UPI handle is handle@psp where the right side is an alphabetic PSP
    // handle, not a domain with a dot. This separates UPI from email.
    if let Some((_, psp)) = s.split_once('@') {
        !psp.contains('.') && psp.chars().all(|c| c.is_ascii_alphabetic()) && psp.len() >= 2
    } else {
        false
    }
}
