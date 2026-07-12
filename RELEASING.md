# Releasing Drishti

One version tag deploys everything, everywhere. Pushing a `v*` tag runs
`.github/workflows/release.yml`, which builds and publishes, in a single run:

| Target | What | Auth |
|---|---|---|
| PyPI | `sarthiai-drishti` (embedded PyO3 wheels + sdist; imported as `drishti`) | trusted publishing, environment `pypi` |
| PyPI | `sarthiai-drishti-sdk` (pure remote client SDK; imported as `drishti_sdk`) | trusted publishing, environment `pypi-drishti-sdk` |
| npm | `sarthiai-drishti-sdk` (remote client SDK, ESM + CJS + types) | `NPM_TOKEN` secret |
| crates.io | `sarthiai-drishti-regex`, `sarthiai-drishti-core`, `sarthiai-drishti-models` (in that order) | `CARGO_REGISTRY_TOKEN` secret |
| Docker Hub | `sarthiai/drishti` (multi-arch server image) | `DOCKERHUB_USERNAME` / `DOCKERHUB_TOKEN` secrets |

Coordinates: GitHub `SarthiAI/Drishti`, Docker Hub org `sarthiai` (Docker Hub is
lowercase). All components share the tag version; the `guard` job fails the run
if any manifest disagrees with the tag.

## One-time setup (owner, not code)

Do this once before the first `v*` tag.

### PyPI trusted publishers (two, distinct environments)

PyPI silently rejects a second pending publisher with the same (Owner, Repo,
Workflow, Environment) tuple, so the two projects MUST use different environment
names. Create both projects, then add a trusted publisher to each:

- Project `sarthiai-drishti`: Owner `SarthiAI`, Repo `Drishti`, Workflow
  `release.yml`, Environment `pypi`.
- Project `sarthiai-drishti-sdk`: Owner `SarthiAI`, Repo `Drishti`, Workflow
  `release.yml`, Environment `pypi-drishti-sdk`.

(The plain names `drishti` and `drishti-sdk` are already taken on PyPI, hence the
`sarthiai-` prefix. The import names are unaffected: `import drishti` and
`import drishti_sdk`.)

In the GitHub repo, create two environments named `pypi` and `pypi-drishti-sdk`.

### npm

- `sarthiai-drishti-sdk` is an unscoped package published under your npm account
  (no org needed).
- Create an npm automation token for that account and store it as the repo secret
  `NPM_TOKEN`.

### crates.io

- Create a crates.io API token and store it as the repo secret
  `CARGO_REGISTRY_TOKEN`.
- The names `sarthiai-drishti-regex`, `sarthiai-drishti-core`, and
  `sarthiai-drishti-models` must be free on crates.io. The first successful
  publish claims them. (The crates are imported in Rust as `drishti_regex`,
  `drishti_core`, `drishti_models`; only the published names carry the prefix.)

### Docker Hub

- Create the `sarthiai/drishti` repository.
- Store `DOCKERHUB_USERNAME` and a Docker Hub access token as `DOCKERHUB_TOKEN`.

## Cutting a release

1. Bump the version to `X.Y.Z` in every manifest so they match:
   - `Cargo.toml`: `[workspace.package] version`, and the two internal versions
     under `[workspace.dependencies]` (`drishti-regex`, `drishti-core`).
   - `pyproject.toml`: the embedded `drishti` wheel.
   - `clients/python/pyproject.toml`: `drishti-sdk`.
   - `clients/node/package.json`: `@sarthiai/drishti-sdk`.
2. Update `CHANGELOG.md`.
3. Commit.
4. Tag and push: `git tag vX.Y.Z && git push origin vX.Y.Z`.
5. The release workflow runs. `guard` verifies the tag matches all manifests,
   then every publish job fans out. crates.io publishes in dependency order with
   a pause between each so the index propagates; if a crates.io step still races,
   re-run just that job.

## Local verification before tagging

```
cargo build --release
cargo package --no-verify -p sarthiai-drishti-regex -p sarthiai-drishti-core -p sarthiai-drishti-models
cd clients/python && uv build && ls dist && cd ../..
cd clients/node   && npm install && npm run typecheck && npm run build && cd ../..
```

## Verifying a release

```
pip install sarthiai-drishti     && python -c "import drishti; print(drishti.__version__)"
pip install sarthiai-drishti-sdk && python -c "import drishti_sdk; print(drishti_sdk.__version__)"
npm view sarthiai-drishti-sdk version
docker run --rm sarthiai/drishti --help
```
