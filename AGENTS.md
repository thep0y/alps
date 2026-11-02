# AGENTS.md

This repository uses a branch-only contribution workflow. These rules apply to human contributors and to coding agents.

## Non-negotiable rules

1. `main` is integration-only. Never commit or push directly to `main`.
2. Every code, test, configuration, documentation or CI change must be made on a feature branch.
3. A feature branch must be created from `main`. Never create one from another feature branch, a release branch, a tag, or an arbitrary commit.
4. All changes reach `main` only through a pull request whose base branch is `main`.
5. Do not rewrite `main` history: no force-push, no rebase, no history edits.

## Required workflow

### 1. Start from an up-to-date `main`

```bash
git switch main
git pull --ff-only origin main
git switch -c feat/short-description
```

If the working tree is not clean, stop and resolve it before creating the branch.

### 2. Make a focused change

Use a descriptive lowercase branch name:

- `feat/...` for features
- `fix/...` for bug fixes
- `docs/...` for documentation
- `test/...` for test-only changes
- `refactor/...` for behavior-preserving changes
- `chore/...` for maintenance
- `hotfix/...` for urgent fixes; it still starts from `main`

Keep the branch limited to one logical change. Add or update tests and documentation with the implementation.

Before publishing, run:

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo build --release --locked
```

### 3. Publish through a pull request

```bash
git push -u origin feat/short-description
gh pr create --base main --head feat/short-description
```

The pull request base is always `main`. Do not open stacked pull requests or target another feature branch.

### 4. Keep the feature branch current

When `main` advances, update the feature branch by merging or rebasing `origin/main`:

```bash
git fetch origin
git rebase origin/main
```

The branch's origin remains `main`; do not rebase or merge it onto another feature branch.

## Agent checklist

Before the first edit:

```bash
git branch --show-current
```

If the current branch is `main`, create a feature branch first. Never make an edit that is intended to be committed while on `main`.

Also inspect the branch point:

```bash
git log --oneline --decorate -5
git merge-base --is-ancestor origin/main HEAD
```

The second command must succeed, and the recent history must show that the branch was cut from `main`, not from another feature branch. A successful ancestor check alone is not sufficient.

Agents must:

- refuse to commit directly to `main`;
- avoid pushing tags or release artifacts from a feature branch;
- never force-push `main`;
- report the branch name and pull request base in the final summary;
- leave unrelated worktree changes untouched.

If a user asks for a direct commit to `main`, explain this policy and move the change to a feature branch instead.

## Automated release exception

The `release` workflow may commit a version-only bump to `main`. That commit is limited to `Cargo.toml` and `Cargo.lock` (and generated release metadata); it must not contain source, test, configuration or documentation changes. Any such change still requires a feature branch and pull request.

## Enforcement

- Branch protection on `main` should reject direct pushes and require pull requests and CI.
- Reviewers must verify the pull request base is `main` and that the branch was cut from `main`.
- CI runs on pull requests and on `main` after merge.
- A change that bypasses this workflow is not ready to merge.
