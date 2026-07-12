# Security policy

## Reporting a vulnerability

If you find a security issue in Drishti, please report it privately rather than
opening a public issue. Use GitHub's private vulnerability reporting: open the
repository's [Security tab](https://github.com/SarthiAI/Drishti/security) and
click "Report a vulnerability". Please include:

- a description of the issue and its impact,
- steps to reproduce, and
- any affected versions.

You can expect an acknowledgement within a few days. Please give us reasonable
time to release a fix before public disclosure.

## Scope

Drishti is a content-safety scanner. It reports scores; it does not make policy
decisions. Its detection guarantees, and their limits, are described in the
threat model in the [README](README.md). In particular, evasion of a specific
classifier, jailbreaks that do not use injection patterns, and non-English
content are out of scope of the detection contract, and are not security issues
in themselves.

Security issues we do want to hear about include, for example:

- a way to make Drishti load or execute a model other than the one configured,
- a path that bypasses SHA-256 verification when a hash is configured,
- a crash or resource exhaustion triggerable by crafted input,
- leakage of secrets (bearer tokens, model paths) through logs or responses.

## Supported versions

Drishti is pre-1.0. Security fixes are made against the latest release.
