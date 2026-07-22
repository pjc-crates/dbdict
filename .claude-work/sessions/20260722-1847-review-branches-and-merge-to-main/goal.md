# review branches and merge to main

> **Revision note (2026-07-22):** the first draft of this file claimed the repo
> held two unrelated histories and that the five upstream branches carried
> 25–110 orphan commits each. That was wrong. The local clone was **shallow**
> (`.git/shallow` grafted `c1de1c8` as a fake root), which made `git log`,
> `git rev-list --max-parents=0`, and `git merge-base` all report a false
> topology consistently. `git fetch --unshallow` has been run — additive, no
> history rewritten. Everything below reflects the real repository.

## problem

The remote carries 7 branches and the layout no longer describes the project.

```
1172b9b "Initial commit" 2026-03-17  (single root — one history)
  │
  ├── … upstream data-dict trunk …
  │     ├── yaml-schema           1 unique commit,  88 behind main
  │     ├── more-constraints      1 unique commit,  52 behind
  │     ├── feature/vscode        6 unique commits, 35 behind
  │     ├── uniqueness            2 unique commits,  7 behind
  │     └── d03-enum-validation   2 unique commits,  4 behind
  │
  └── main  112 commits, tip c1de1c8 (upstream PR #106, 2026-07-02)
        └── duckdb-source  154 commits (= main + 42 by Peter Crosbie)
```

Two facts drive the session:

1. **`main` is still upstream.** Its tip is tidyverse `data-dict` at PR #106.
   The actual project — the DuckDB pivot, 42 commits across 8 work sessions —
   lives only on `duckdb-source`. `main` is the GitHub default branch, so the
   repo's landing page presents upstream's tool, not dbdict. The repo has just
   moved to `pjc-crates/dbdict`, which points at publishing; the default branch
   should be the project before that happens.

2. **Five upstream feature branches are inert here.** Together they hold 12
   unique commits, all authored upstream (Hadley Wickham, Gábor Csárdi). They
   *can* merge — one shared history, no `--allow-unrelated-histories` needed —
   but CLAUDE.md is explicit that this fork is "deliberately diverged — not
   tracking upstream, not aiming for cross-backend portability," and three of
   the five are already superseded by dbdict's own DuckDB-native work.

## success criteria

- `main` is the project: identical to today's `duckdb-source` (154 commits),
  still the GitHub default branch.
- The five upstream branches are gone from the remote, with every commit still
  reachable via a permanent `archive/<name>` tag on the remote.
- Each archive tag verified to resolve to the branch's pre-deletion SHA —
  checked *after* the delete, not only before.
- `duckdb-source` deleted local and remote; checkout on `main` tracking
  `origin/main`, clean tree.
- `cargo test --workspace` passes (415 tests) from the `main` checkout.
- Remote branch list is exactly: `main`.

## scope

- in:
  - tag the 5 upstream branches `archive/<name>`, push tags, verify
  - delete those 5 branches from the remote
  - fast-forward `main` to `duckdb-source` (verified strict ancestor — no
    merge commit, no force)
  - delete `duckdb-source` local and remote; move checkout to `main`
  - verify tags resolve, tests pass, branch list clean

- out:
  - **porting any upstream code.** See the open question below — if the answer
    is "salvage something", that becomes its own session, not this one.
  - rewriting, squashing, or re-rooting history
  - crates.io publishing
  - the untracked `research/` dir (Claude Code startup notes, unrelated to
    dbdict — still awaiting a gitignore/commit/delete decision from the
    2026-07-19 checkpoint)
  - the 7 `.claude-work/` files still naming `pjc-wspace` — dated records,
    deliberately left

## open question — reconsider before executing

The "archive and delete" decision was taken when I had mis-reported these
branches as 217 commits of upstream work. The real figure is **12 commits
across ~67 files**, which makes a salvage review cheap rather than a session
of its own. Three are worth a deliberate decision rather than a default:

- `feature/vscode` (6 commits) — LSP server behind a hidden subcommand plus a
  VS Code extension. **The only branch not duplicated by dbdict's own work**;
  the project has no editor integration.
- `d03-enum-validation` (2) — upstream's "D03" is *enum value validation*.
  dbdict's D03 is the *unique column check*. Same identifier, different check.
  Worth knowing before either name reaches published docs, regardless of
  whether the code is wanted.
- `uniqueness` (2) — upstream's exact D02 uniqueness for **Parquet**; dbdict
  already has D02 for DuckDB. Most likely genuinely redundant.

Archiving as tags keeps all of this recoverable either way, so this is a
question of whether to look now or later — not a question of losing anything.

## constraints

- **Deleting a remote branch is the only irreversible step.** Tags must be
  pushed and confirmed present on the remote *before* any delete, and
  re-confirmed after.
- No force-push. The FF is a plain push because `main` is a strict ancestor of
  `duckdb-source` — verified with `git merge-base --is-ancestor` after the
  unshallow. If that check ever fails, stop.
- Verified preconditions (2026-07-22, post-unshallow): `main` has no branch
  protection and no rulesets; no open PRs; repo has 0 tags, so the `archive/`
  namespace is free.
- `duckdb-source` cannot be deleted while checked out, and must not be deleted
  before `main` has been fast-forwarded.
- GitHub's default branch is already `main`, so no default-branch switch is
  needed and deleting `duckdb-source` won't orphan the landing page.
- **Verify topology claims against a non-shallow clone.** This session already
  produced one wrong plan from shallow-clone artifacts.
