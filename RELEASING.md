# Releasing Drishti

A release is driven entirely by pushing a `v*` git tag. The tag triggers two
workflows: `wheels.yml` builds the wheels and sdist and publishes them to PyPI,
and `docker.yml` builds the multi-arch image and pushes it to Docker Hub.

## One-time setup

These steps are done once, in the project accounts. They are not code.

### PyPI (trusted publishing)

1. Create the project `drishti` under the `sarthiai` PyPI account (a first manual
   upload, or a pending publisher).
2. In the project settings, add a trusted publisher:
   - Owner: `sarthiai`
   - Repository: `drishti`
   - Workflow: `wheels.yml`
   - Environment: `pypi`
3. In the GitHub repo, create an environment named `pypi` (Settings, Environments).
   No token is stored; publishing uses OIDC.

Note: PyPI requires the (Owner, Repo, Workflow, Environment) tuple to be unique
per project. If a second package is ever published from this repo, give it a
different environment name.

### Docker Hub

1. Create the repository `sarthiai/drishti` on Docker Hub.
2. Create an access token (Account Settings, Security).
3. In the GitHub repo, add secrets:
   - `DOCKERHUB_USERNAME`
   - `DOCKERHUB_TOKEN`

## Cutting a release

1. Update `CHANGELOG.md`: move items from Unreleased into a new version section.
2. Bump the version in `Cargo.toml` (`workspace.package.version`) and
   `pyproject.toml`.
3. Commit.
4. Tag and push:

   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```

5. Watch the `wheels` and `docker` workflows. On success:
   - wheels and sdist are on PyPI (`pip install drishti`),
   - the image is on Docker Hub (`docker pull sarthiai/drishti`).

## Verifying a release

```bash
pip install drishti && python -c "import drishti; print(drishti.__version__)"
docker run --rm sarthiai/drishti --help
```
