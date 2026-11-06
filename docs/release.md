# CI Caching and Release Process

## 1. Cache warming on main

Cache warming is a separate workflow from pull-request validation:

- [`.github/workflows/warm-cache.yml`](../.github/workflows/warm-cache.yml) runs on pushes to `main` and is the only cache writer.
- [`.github/workflows/pr.yml`](../.github/workflows/pr.yml) runs the lint, test, release-build, MSRV, cross, audit and Docker jobs on pull requests and restores caches read-only (`save-if: false`).

The cache keys are:

- **debug cache**: `shared-key: debug`, warmed by `cargo test --all-features`.
- **release cache**: `shared-key: release`, warmed by `cargo build --release --locked`.
- **MSRV cache**: `shared-key: msrv`.
- **cross cache**: `shared-key: cross-release`.
- **Docker cache**: BuildKit GHA scope `alps-docker`, warmed by the `docker` job in `warm-cache`.

`warm-cache` saves only for `main`:

```yaml
save-if: ${{ github.ref == 'refs/heads/main' }}
```

Pull requests and feature branches restore the main caches but never overwrite them. The two profiles use different `shared-key` values because the release profile uses thin LTO and `codegen-units = 1`; mixing debug and release artifacts would cause constant cache misses.

Keeping `push` and `pull_request` in separate workflows is intentional: a feature-branch push followed by a pull-request synchronization would otherwise run every validation job twice.

The Docker cache uses a separate GitHub Actions scope (`alps-docker`). Only the `warm-cache` Docker job writes it; the Docker job in `pr.yml` restores it without `cache-to`, so no PR-scoped caches are created.

> Forks do not receive the base repository's Actions caches by design; their first run must download dependencies.

### PR labels and merge

- The `type-label` job in `pr.yml` applies exactly one `type:*` label from the head branch prefix (`feat/`, `fix/`, `docs/`, ...), falling back to the PR title's Conventional Commits prefix. It removes conflicting `type:*` labels.
- The final `automerge` job runs only when the PR has the `automerge` label. On a normal code event it runs after all seven validation jobs and merges the checked head with a squash merge. On a label-only event the validation jobs are skipped; the job merges immediately only if the current checks are already successful, otherwise it exits and the running code workflow performs the merge when checks finish.
- The `automerge` job removes the label after the merge. The repository's automatic head-branch deletion remains enabled.
- Label changes do not rerun validation jobs. Since `labeled` must be in the workflow trigger, a label-only run still appears, but every validation job is skipped and the automerge job only does work for the `automerge` label.

No separate polling job is used.

## 2. Release workflow

`.github/workflows/release.yml` is a manual `workflow_dispatch` workflow with three inputs:

| Input | Values | Meaning |
|---|---|---|
| `bump` | `patch`, `minor`, `major` | Target core version bump |
| `kind` | `stable`, `alpha`, `beta` | Release type |
| `dry_run` | boolean | Run validation and builds without committing, tagging or publishing |

It is split into four jobs. `prepare` computes and validates the next version. `build-x86_64` and `build-aarch64` then run in parallel on separate runners and upload their tarballs as workflow artifacts. `publish` downloads both artifacts, regenerates the checksums, commits the version change to `main`, creates an annotated `v<version>` tag, and publishes a GitHub release with categorized notes. Alpha/beta versions are marked as pre-releases.

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

The builds are parallelizable because each runner starts from the same pinned commit and deterministically applies the same bump. The version is not committed until both builds pass, so a failed architecture does not leave a version bump or tag behind.

### Release notes

`scripts/release_notes.py` generates the release body before the release is published:

1. The previous non-draft GitHub release is selected (the current tag is skipped if it already exists); with no previous release, the whole history is used.
2. Commits in `<previous>..HEAD` are resolved to merged pull requests through the GitHub API.
3. Each PR title is classified by its Conventional Commits prefix:

| Section | PR title types |
|---|---|
| ⚠ Breaking Changes | `!` before the colon, or `BREAKING CHANGE:` in the PR body |
| 🚀 Features | `feat`, `feature` |
| 🐛 Bug Fixes | `fix`, `hotfix`, `bugfix` |
| ⚡ Performance | `perf` |
| ♻ Refactoring | `refactor` |
| 📝 Documentation | `docs`, `doc` |
| 🧪 Tests | `test`, `tests` |
| 🏗 Build | `build` |
| ⚙ CI | `ci` |
| 🧹 Chores | `chore`, `deps`, `release` |
| ⏪ Reverts | `revert` |
| 📦 Other Changes | anything without a recognized prefix |

Breaking entries appear only in the Breaking Changes section. The notes show the cleaned PR title, PR link and author, plus a Full Changelog compare link. Direct commits without an associated merged PR are omitted.

The generated file is passed to `gh release create --notes-file`; it is not uploaded as an asset. If notes generation fails, the workflow stops before committing or publishing.

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

# Preview categorized notes for the next tag (requires gh auth)
python3 scripts/release_notes.py --tag v0.1.2-alpha.1
```
