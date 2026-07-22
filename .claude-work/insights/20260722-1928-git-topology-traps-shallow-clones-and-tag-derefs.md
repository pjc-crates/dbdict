---
created: 2026-07-22T19:28:14+12:00
title: git topology traps — shallow clones and tag derefs
tags: [git, github, gotcha, verification, sourcing]
source: /ws done
---

## a shallow clone lies consistently and plausibly

A shallow clone doesn't announce itself — it lies consistently and plausibly.
`git log main` showed 1 commit, `git rev-list --max-parents=0` reported
`c1de1c8` as a root, and `git merge-base` returned empty. Three independent
commands all agreed on a false topology, because `.git/shallow` grafts the
boundary commit to look parentless to every read path.

The check that would have caught it up front is
`git rev-parse --is-shallow-repository`. The tell that actually caught it was a
*contradiction in the record*: the state file cited HEAD `6631195` while the
four commits beneath it matched today's SHAs exactly. A rewrite changes every
descendant SHA, so partial agreement was impossible under any rewrite story —
which meant the model of the history was wrong, not the file.

Consequence for this repo: what looked like "two unrelated histories with
25–110 orphan commits per branch" was actually one history rooted at
`1172b9b`, `main` at 112 commits, and 12 unique commits across five ordinary
feature branches. An entire session plan was drafted on the false reading
before `git fetch --unshallow` corrected it.

## annotated tags need `^{}` to compare against a commit SHA

Verifying archive tags derefs each tag with `^{}` rather than reading the ref
directly, and that distinction is load-bearing. An annotated tag creates a
*tag object* with its own SHA; `refs/tags/archive/uniqueness` points at the tag
object, not at the commit. Comparing that SHA against the recorded commit SHA
mismatches on every tag — correct tags reported as failures.

`git ls-remote --tags` publishes both entries, so a real remote check sees N
tag objects and N dereferenced peels. This is also why a restore command must
be `archive/x^{}:refs/heads/x`: pushing the tag object itself would create a
branch pointing at a tag, not at the commit.

## nested tag names are legal — the constraint is a directory/file conflict

Nearly shipped a wrong claim in a plan: that git would refuse
`archive/feature/vscode` and the name needed mangling to
`archive/feature-vscode`. Testing in a scratch repo showed the nested tag is
perfectly legal alone. Git's refs are literally files under `.git/refs/`, so
the constraint is a **directory/file conflict** — `refs/tags/archive/feature`
can't be a file while `refs/tags/archive/feature/` is a directory. It fires in
both creation orders, but only when both names exist. Since nothing creates
`archive/feature`, the nested name is fine and tags can mirror branch names
with no mapping to remember later.

## `gh repo create --push` pushes HEAD only, and fails late

From the gh source (`pkg/cmd/repo/create/create.go` @ v2.92.0):

- Line 681-682: `if opts.Push && repoType == working { GitClient.Push(ctx, baseRemote, "HEAD") }`
  — for a non-bare repo `--push` pushes **HEAD only**, i.e. the current branch.
  Other remote branches are silently left behind. Only a *bare* repo gets
  `push --mirror` (line 692-693).
- Line 738: `sourceInit` runs a plain `git remote add`, which fails when
  `origin` already exists — and it fails *after* the GitHub repo has already
  been created, leaving an orphan remote repo to clean up.

## repo rename and org rename share a verb, not an operation

`gh repo rename` changes only the segment *after* the slash; ownership changes
are exclusively the transfer path (`POST /repos/{owner}/{repo}/transfer`, no
`gh` subcommand). The one case where a rename *does* move the owner segment is
renaming the **organization itself**, which rewrites the owner for every repo
it holds — dragging unrelated repos along. Repo-level rename and org-level
rename are unrelated operations that happen to share a verb.

Transfer preserves stars, watchers, issues, and PRs, and sets up redirects — a
create-and-push drops all four.

## scope a bulk rename to the pattern, not the bare token

When rewriting `pjc-wspace` → `pjc-crates`, the substitution was scoped to
`pjc-wspace/dbdict` and `pjc-wspace.github.io` rather than the bare org name,
because the working directory is literally `/home/pjc/pjc-wspace/dbdict`. The
org name and a parent directory name collide. A bare substitution would be
correct only by luck — the moment a fixture or doc embeds an absolute path, it
silently corrupts it. Enumerate the distinct match contexts first
(`grep -o '.\{0,18\}TOKEN.\{0,14\}' | sort | uniq -c`), then scope to the
shapes that actually appear.

## rewriting a constant and its snapshots in one pass avoids the insta fixpoint loop

The URL sweep needed no insta accept-and-re-run loop, which
`20260707-1159-snapshot-url-exposure-and-fixpoint-acceptance` warns is normally
required. Rewriting `LEARN_MORE_URL` and the 37 `.snap` files in a *single*
pass kept producer and expectation in lockstep, so snapshots never went
pending. The fixpoint loop is only needed when the constant changes and the
snapshots have to catch up — sed across both sides at once sidesteps it.

## types-only DDL is the decision the whole project hangs off

Re-derived while reading the session record. Generated schemas deliberately
never emit `PRIMARY KEY`/`NOT NULL`/`UNIQUE`, because DuckDB's performance
guide says to avoid PK constraints for bulk loading. That forced a
declare-then-check model — and every subsequent session existed because of it.
D03, D04, and D05 are all "the database won't enforce this, so we must query
for violations." The dummy-data generator then inverted the same model: it
satisfies D01–D05 by index arithmetic rather than rejection sampling, using
`validate-data` as a built-in oracle.
