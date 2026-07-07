# summary: fork branding

started: 2026-07-07 10:34
closed: 2026-07-07T13:59:29+12:00

## goal

dbdict is a deliberate fork of tidyverse `data-dict`, but upstream
branding leaked everywhere a user or diagnostic pointed outward: the
`LEARN_MORE_URL` fix-it constant, ~10 fixtures/examples embedding the
upstream URL, site/CNAME claiming their domain, _quarto.yml carrying
their site-url/repo-url and Plausible analytics, an upstream-voiced
site/index.md, and a stale upstream-flavoured .claude/claude.md. Last
item blocking site publishing; its decisions (name, URL) leak into
everything written afterwards.

## what was accomplished

### phase 1: mechanical URL sweep (commit da06873)
- `LEARN_MORE_URL` → https://github.com/pjc-wspace/dbdict (feeds S09's
  fix-it suggestion)
- all test/fixture yaml swept from the upstream URL (8 sources, 2
  fixture files)
- ~20 snapshots regenerated; acceptance ran as a fixpoint loop with a
  machine-checked diff whitelist (URL swap or insta metadata only)
- crates/ contains zero upstream references; 290 tests green

### phase 2: site + meta rebrand, index.md rewrite (commit 4d3ec84)
- site/CNAME deleted (with its _quarto.yml resources entry)
- _quarto.yml: Plausible analytics dropped; title "dbdict.yaml";
  site-url https://pjc-wspace.github.io/dbdict/; repo-url and nav →
  github.com/pjc-wspace/dbdict
- site/spec.md `$learn_more` recommendation → the fork repo URL
- site/index.md rewritten dbdict-first: rich format lead, Lineage
  section crediting upstream (MIT) and framing the five upstream
  examples as legacy-format examples, Why bullets updated
  (single-engine argument replaces parquet-first), Direction section
  states the fork roadmap (dummy data, Python/Julia codegen, doc
  generation)
- .claude/claude.md fully rewritten (user-requested extension):
  describes dbdict as it stands — crate list, CLI subcommands, module
  layout, fixture dirs, schema files all re-verified against the repo —
  names the original project only in a Lineage paragraph noting the
  decision to move away, defers comment policy to the root CLAUDE.md,
  drops the stale nanoparquet instruction

## key decisions

- canonical `$learn_more` URL is the GitHub repo — always valid, no
  site publishing required, switchable to a Pages URL later (user)
- rebrand the site but publish later; publishing is a separate
  decision (user)
- index.md gets a dbdict-first rewrite with a lineage note, not a
  light touch (user)
- sweep all fixtures/examples to the new URL — S09 checks presence
  only, so behaviour is unchanged (user)
- .claude/claude.md mentions the original project only as lineage,
  with the move-away decision noted (user, mid-phase extension)
- upstream references surviving by design: README credit, LICENSE
  copyright, root CLAUDE.md fork note, index.md Lineage — attribution
  stays (MIT)

## insights captured

- .claude-work/insights/20260707-1159-snapshot-url-exposure-and-fixpoint-acceptance.md
- .claude-work/insights/20260707-1357-fork-rebrand-surface-and-inherited-process-files.md
