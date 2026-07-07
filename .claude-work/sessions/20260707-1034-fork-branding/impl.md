# implementation: fork branding

## phases

### phase 1: mechanical URL sweep (code, fixtures, snapshots) — DONE 2026-07-07T11:59:17+12:00
- [x] crates/dbdict/src/validate_spec.rs: `LEARN_MORE_URL` →
      `https://github.com/pjc-wspace/dbdict` (the constant feeds S09's
      fix-it replacement; comment updated to say it's the fork's repo)
- [x] sweep `http://data-dict.tidyverse.org/` → the new URL across
      crates/ — inline `indoc!` yaml in tests (rich.rs, rich_meta.rs,
      rich_data.rs, common/mod.rs, e2e_validate_meta.rs, cli.rs) and
      fixture files (tests/fixtures/spec/clean-two-tables.yaml,
      s12-s13-valid-ok.yaml)
- [x] regenerate the snapshots that embed the URL: run tests, review
      .snap.new diffs — every hunk must be a URL-text-only change, no
      span/caret movement except column shifts on the `$learn_more`
      line itself — then accept via mv
- also: only ~20 of the 36 URL-bearing snapshots changed — a snapshot
      embeds the `$learn_more` line only when the rendered context
      window reaches it; the rest carried the URL via echoed fixtures
      whose excerpts never show line 2. acceptance ran as a fixpoint
      loop (no cargo-insta installed) with a machine-checked diff
      whitelist (URL swap or insta `assertion_line` metadata only)
- also: cargo fmt reflowed one `RICH_HEADER` const in tests/rich.rs —
      the longer URL pushed the line over rustfmt's limit
- **verify:** `cargo test --workspace` green;
  `cargo clippy --workspace --all-targets` + `cargo fmt --all --check`
  clean; `grep -rn "tidyverse" crates/` returns nothing

### phase 2: site + meta rebrand, index.md rewrite
- [ ] delete site/CNAME (claims data-dict.tidyverse.org — not ours)
- [ ] site/_quarto.yml: drop the Plausible analytics script block;
      repo-url and the nav github link →
      https://github.com/pjc-wspace/dbdict; site-url →
      https://pjc-wspace.github.io/dbdict/
      (Inferred: the natural Pages URL for this repo — harmless until
      publishing, revisit when the publish decision is made)
- [ ] site/spec.md:34: the `$learn_more` recommendation prose points at
      the new URL
- [ ] site/index.md: dbdict-first rewrite — lead with the rich
      DuckDB-native `dbdict.yaml` format (typedefs, exact type
      round-trip, validate-meta/validate-data, ddl) and the fork
      rationale; short lineage note crediting tidyverse `data-dict`;
      legacy `data-dict.yaml` path mentioned as preserved; keep only
      upstream concepts that still apply; examples links checked
      against site/examples/ so none are dead
      — draft reviewed by user before the phase closes
- [ ] .claude/claude.md: the `site/` bullet describes upstream's
      published site and a `.github/workflows/publish-site.yaml` that
      doesn't exist in the fork — rewrite the bullet to match reality
      (site is unpublished; publishing is a future decision)
- [ ] README.md: no change — the fork-credit note (line 10) is the one
      sanctioned upstream reference; confirm it survived the sweep
- **verify:** `grep -rni "tidyverse" . --exclude-dir=target
  --exclude-dir=.git --exclude-dir=.claude-work` returns only the
  README credit and CLAUDE.md's fork note; `cargo test --workspace`
  still green; quarto render skipped (quarto not installed — noted,
  best-effort per goal)
