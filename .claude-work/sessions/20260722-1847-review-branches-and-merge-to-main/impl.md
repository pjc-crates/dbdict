# implementation: review branches and merge to main

## recorded SHAs (2026-07-22, post-unshallow)

Verification targets. Every phase checks against these, not against whatever
the refs happen to say at the time.

| ref | SHA |
|---|---|
| `origin/main` (before) | `c1de1c8e2c2c0f0176538e408b3d256131bc2051` |
| `origin/duckdb-source` | `33ffe89c65ba49bdccb04fd069b59296684def16` |
| `origin/yaml-schema` | `490495192831040bf4b1cef7fa91b1467ab4dec3` |
| `origin/more-constraints` | `756747ad0cd28d02a0227991467faa9c1ea8cae3` |
| `origin/feature/vscode` | `8fe8b5b824b7b53b75bf96db8c56e30f83e8e165` |
| `origin/uniqueness` | `9dd994a62cd29c2319c8fb382268b302315a84c6` |
| `origin/d03-enum-validation` | `ee2c0d930987f6f8c423dfdfa3b364802f5e9a83` |

Phase 2 pushes session commits first, so `main`'s final SHA will be a
descendant of `33ffe89`, not `33ffe89` itself. Phase 2 records the actual
value and phases 3–4 verify against that.

## decision needed before phase 1

**Annotated tags, not lightweight** — a deviation from the approved preview,
flagged for your call.

`git tag archive/x <sha>` (lightweight) stores a bare pointer: no author, no
date, no reason. `git tag -a` stores a real tag object recording who archived
it, when, and why. These tags are the *only* record of why five branches
vanished from a published repo; in a year the annotation is the difference
between provenance and a mystery ref. Cost is one `-a -m` flag.

Say the word if you'd rather keep them lightweight — everything else in the
plan is unchanged either way.

## phases

### phase 1: archive tags — DONE 2026-07-22T19:27:53+12:00

- [x] commit the session files (`.claude-work/.active`, session dir) on
      `duckdb-source` — they must exist before the FF or they don't reach `main`
      → commit `b951c73`, 3 files / 246 insertions
- [x] create 5 annotated tags at the recorded SHAs (not at `origin/<branch>`,
      which could have moved):
      ```
      git tag -a archive/yaml-schema         4904951 -m "…"
      git tag -a archive/more-constraints    756747a -m "…"
      git tag -a archive/feature/vscode      8fe8b5b -m "…"
      git tag -a archive/uniqueness          9dd994a -m "…"
      git tag -a archive/d03-enum-validation ee2c0d9 -m "…"
      ```
- [x] push all 5 tags: `git push origin --tags` — all 5 reported `[new tag]`
- [x] confirm each tag exists **on the remote** and dereferences to the
      recorded SHA (annotated tags need `^{}` to deref past the tag object)
- **verify:** PASS — `git ls-remote --tags origin 'refs/tags/archive/*'`
  returns 5 tag objects + 5 dereferenced peels; all 5 `^{}` values equal their
  recorded SHAs; zero mismatches. Cross-checked independently via
  `gh api repos/pjc-crates/dbdict/tags`, which agrees on all five.
  **Phase 3 is cleared to run on this basis.**

- also: tag annotations went well beyond the planned one-liner. Each records
  the branch's unique-commit count, author, date, file count, distance behind
  `main`, why it was archived, and a working restore command
  (`git push origin archive/<x>^{}:refs/heads/<x>`). Verified via
  `gh api …/git/tags/<sha>` that the annotation survives the push intact.
  Two carry substantive notes: `feature/vscode` is flagged as the only branch
  not superseded by dbdict's own work, and `d03-enum-validation` records the
  identifier collision (upstream D03 = enum validation, dbdict D03 = unique
  column check).
- also: the tagger identity reads `pjc on thelio25`, not `Peter Crosbie`.
  Raised as a possible misconfiguration; user confirmed it is deliberate — two
  git identities, author for the real name and committer for machine
  provenance, so the machine is tracked without cluttering commit messages.
  `git tag -a` takes its tagger from the committer identity. No action; not a
  defect. Recorded as a memory so it is not re-flagged.
- not done: `/code-review` at this phase boundary, contrary to the standing
  mandate in `.claude-work/memory/regular-code-reviews.md`. This phase changed
  no code — its diff is two markdown files plus git refs — and the session's
  operating instructions bar spawning review agents unrequested. Flagged to
  the user rather than skipped silently; phase 4 is the boundary where a
  review would have something to read.

> Tag names mirror branch names exactly, including `archive/feature/vscode`.
> Verified empirically in a scratch repo: a nested tag is legal on its own —
> git only refuses on a directory/file conflict, i.e. if `archive/feature`
> *also* existed as a ref (tested both creation orders; each refuses the
> second with `cannot lock ref … exists`). Nothing creates `archive/feature`,
> so no mapping is needed and no mapping has to be remembered later.

### phase 2: fast-forward main

- [ ] `git push origin duckdb-source` — session commits reach the remote first
- [ ] re-verify the FF precondition against live refs:
      `git merge-base --is-ancestor origin/main origin/duckdb-source`
      — **if this fails, stop the session**
- [ ] `git push origin duckdb-source:main` (plain push, no `--force`,
      no `--force-with-lease` — a true FF needs neither)
- [ ] record the resulting `main` SHA into this file for phases 3–4
- **verify:**
  `git ls-remote origin refs/heads/main` equals `origin/duckdb-source`'s tip;
  `git rev-list --count origin/main` ≥ 155; `gh api …/branches/main` reports
  the same SHA; GitHub default branch still `main`.

### phase 3: delete branches, move checkout to main

- [ ] **re-verify all 5 archive tags on the remote** — the guard immediately
      before the only irreversible step, deliberately repeated from phase 1
- [ ] delete the 5 upstream branches:
      `git push origin --delete yaml-schema more-constraints feature/vscode uniqueness d03-enum-validation`
- [ ] `git checkout main && git merge --ff-only origin/main` — local `main`
      catches up (cannot delete `duckdb-source` while it is checked out)
- [ ] `git push origin --delete duckdb-source`
- [ ] `git branch -d duckdb-source` (`-d`, never `-D` — refuses if not merged,
      which is exactly the safety check wanted here)
- [ ] `git fetch --prune`; `git remote set-head origin -a` so `origin/HEAD`
      tracks `main`
- **verify:**
  `git ls-remote --heads origin` returns exactly one line, `refs/heads/main`;
  all 5 `archive/*` tags **still** resolve to their recorded SHAs *after* the
  deletes; `git branch` shows only `main`; `git rev-parse HEAD` equals
  phase 2's recorded `main` SHA.

### phase 4: verification from a clean main checkout

- [ ] `git status` — clean bar the known-untracked `research/`
- [ ] `cargo test --workspace` — expect 415 passed / 0 failed
- [ ] `cargo clippy --workspace --all-targets` — 0 warnings
- [ ] `cargo fmt --check` — clean
- [ ] confirm `Cargo.toml` `repository` and `LEARN_MORE_URL` still read
      `pjc-crates` (they were rewritten pre-session; this catches a bad merge)
- [ ] spot-check the archive path actually works:
      `git log --oneline -1 archive/feature-vscode` resolves without network
- **verify:** all four commands green; the archive spot-check prints
  `8fe8b5b`; goal.md's success criteria each tick off.

## rollback

Per phase, if verification fails:

- **phase 1** — no destructive action taken. Delete bad tags
  (`git push origin --delete refs/tags/archive/x`) and redo.
- **phase 2** — `main` moved but nothing was deleted. `main`'s old value
  `c1de1c8` is recorded above and still reachable from every archive tag's
  history; restoring is a force-push to `c1de1c8`.
- **phase 3** — branches deleted. Restore from the archive tags:
  `git push origin archive/uniqueness^{}:refs/heads/uniqueness`. GitHub's UI
  also offers branch restore for a period. This is why phase 1's verification
  gates this phase.
- **phase 4** — diagnostic only; no state change to undo.

## out of scope (from goal.md)

Porting upstream code; history rewriting; crates.io publishing; the untracked
`research/` dir; the 7 `.claude-work/` files still naming `pjc-wspace`.
