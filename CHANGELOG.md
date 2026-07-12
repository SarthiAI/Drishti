# Changelog

All notable changes to Drishti are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project aims to
follow semantic versioning once it reaches 1.0.

## [Unreleased]

### Added

- Three content-safety checks: prompt-injection detection, PII detection and
  redaction, and output-safety classification.
- Three surfaces over one core: the `drishti` CLI, the `drishti-server` HTTP
  service, and the `drishti` Python package, all returning identical results.
- Configurable models with present-or-fetch loading: nothing hardcoded, download
  on first use, optional SHA-256 verification, bring-your-own-model by local path.
- Configuration via TOML with environment-variable and `.env` overrides for every
  value.
- PII regex stage (emails, cards with Luhn, phones, IPs, IBAN, SSN, PAN, Aadhaar,
  UPI, NINO, VAT) and an optional NER stage with an acronym filter.
- Output-safety support for both multi-label and softmax-with-safe-class models.
- The `drishti-eval` harness: precision, recall, and F1 against labelled sets,
  with a JSON report stamped with model hashes, and a validated-versus-experimental
  gate.
- A ready-to-run starter configuration and a reference Docker image.
- Elastic License 2.0.

[Unreleased]: https://github.com/sarthiai/drishti
