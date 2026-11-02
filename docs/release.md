# CI Caching and Release Process

## 1. Cache warming on main

Cache warming is a separate workflow from pull-request validation:

- [`.github/workflows/warm-cache.yml`](../.github/workflows/warm-cache.yml) runs on pushes to `main` and is the only cache writer.
- [`.github/workflows/ci.yml`](../.github/workflows/ci.yml) runs on `pull_request` only; it restores caches read-only (`save-if: false`).

The cache keys are:

- **debug cache**: `shared-key: debug`, warmed by `cargo test --all-features`.
- **release cache**: `shared-key: release`, warmed by `cargo build --release --locked`.
- **MSRV cache**: `shared-key: msrv`.
- **cross cache**: `shared-key: cross-release`.

`warm-cache` saves only for `main`:

```yaml
save-if: ${{ github.ref == 'refs/heads/main' }}
```

Pull requests and feature branches restore the main caches but never overwrite them. The two profiles use different `shared-key` values because the release profile uses thin LTO and `codegen-units = 1`; mixing debug and release artifacts would cause constant cache misses.

Keeping `push` and `pull_request` in separate workflows is intentional: a feature-branch push followed by a pull-request synchronization would otherwise run every validation job twice.

The Docker build uses a separate GitHub Actions cache scope (`alps-docker`).

> Forks do not receive the base repository's Actions caches by design; their first run must download dependencies.

## 2. Release workflow

`.github/workflows/release.yml` is a manual `workflow_dispatch` workflow with three inputs:

| Input | Values | Meaning |
|---|---|---|
| `bump` | `patch`, `minor`, `major` | Target core version bump |
| `kind` | `stable`, `alpha`, `beta` | Release type |
| `dry_run` | boolean | Run validation and builds without committing, tagging or publishing |

It checks out `main`, bumps the version, validates, builds x86_64 and aarch64 binaries, commits the version change to `main`, creates an annotated `v<version>` tag, and publishes a GitHub release with generated notes and tarball/checksum assets. Alpha/beta versions are marked as pre-releases.

### Version bump semantics

`scripts/bump_version.py` updates both `Cargo.toml` and the root `alps` entry in `Cargo.lock`.

| Current | bump | kind | Next | Notes |
|---|---|---|---|---|
| 0.1.0 | patch | stable | 0.1.1 | Normal patch |
| 0.1.0 | patch | alpha | 0.1.1-alpha.1 | Explicit next-patch alpha |
| 0.1.0 | minor | beta | 0.2.0-beta.1 | Explicit next-minor beta |
| 0.1.0 | major | alpha | 1.0.0-alpha.1 | Explicit next-major alpha |
| 0.1.1-alpha.1 | patch | alpha | 0.1.1-alpha.2 | Same core and identifier increments |
| 0.1.1-alpha.1 | patch | beta | 0.1.1-beta.1 | Same core, different identifier resets |
| 0.1.1-alpha.1 | patch | stable | 0.1.1 | Finalizes the pre-release |
| 0.1.1-alpha.1 | minor | stable | 0.2.0 | Bumps the core and drops the pre-release |
| 0.1.1-beta.1 | minor | beta | 0.2.0-beta.1 | Starts beta on the next minor |
| 0.1.1-beta.1 | major | stable | 1.0.0 |  |

`bump` selects the target core; `kind` selects stable/alpha/beta. On an existing pre-release, `patch` keeps the current core (so it can increment, switch the identifier, or finalize with `stable`), while `minor`/`major` move to the next core. A stable version never guesses a pre-release core: `kind = alpha|beta` is always combined with an explicit `bump`.

A stable release after a beta is normally made with `bump = patch`, `kind = stable`.

### Required repository settings

- Actions must be allowed to write repository contents: the workflow declares `permissions: contents: write`.
- Branch protection on `main` may block the automatic commit. Either allow GitHub Actions to bypass the rule for this workflow, or add a `RELEASE_TOKEN` secret (a fine-grained PAT with contents write) and use it for checkout/push and `gh`. The workflow prefers `RELEASE_TOKEN` when present.
- The workflow does not push to protected branches with force and performs no history rewriting.

### Recovery

- If validation/build fails before the commit, `main` is unchanged; rerun the workflow.
- If the commit was pushed but tagging or release creation failed, the version is already on `main`. Create the missing tag/release manually for that commit, or bump again for a new version. Do not rewrite `main`.
- `dry_run` is the safe way to verify a bump and both builds without touching the branch.

### Local equivalent

```bash
# Print the next version without writing
python3 scripts/bump_version.py --bump patch --kind stable
python3 scripts/bump_version.py --bump minor --kind beta
python3 scripts/bump_version.py --bump patch --kind alpha   # existing pre-release

# Update Cargo.toml and Cargo.lock
python3 scripts/bump_version.py --bump minor --kind beta --write
cargo check --locked --all-features
cargo test --all-features
```
