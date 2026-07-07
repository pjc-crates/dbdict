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

### phase 2: site + meta rebrand, index.md rewrite — DONE 2026-07-07T13:57:13+12:00
- [x] delete site/CNAME (claims data-dict.tidyverse.org — not ours)
- also: removed the `resources: CNAME` entry from _quarto.yml with it
      (quarto would fail on a missing resource)
- [x] site/_quarto.yml: drop the Plausible analytics script block;
      repo-url and the nav github link →
      https://github.com/pjc-wspace/dbdict; site-url →
      https://pjc-wspace.github.io/dbdict/
      (Inferred: the natural Pages URL for this repo — harmless until
      publishing, revisit when the publish decision is made)
- also: website title → "dbdict.yaml" and description mentions
      DuckDB-native — consistency with the dbdict-first index.md
- [x] site/spec.md:34: the `$learn_more` recommendation prose points at
      the new URL
- [x] site/index.md: dbdict-first rewrite — lead with the rich
      DuckDB-native `dbdict.yaml` format (typedefs, exact type
      round-trip, validate-meta/validate-data, ddl) and the fork
      rationale; Lineage section credits tidyverse `data-dict` (MIT)
      and reframes the five upstream examples as legacy-format
      examples (all verified present, all v0.1.0); Why bullets kept
      where still true, parquet bullet replaced with the single-engine
      argument; Direction section states the fork roadmap (dummy data,
      Python/Julia codegen, doc generation)
      — draft reviewed by user (approved via /ws done)
- [x] extension (user, 2026-07-07): full .claude/claude.md rewrite —
      describe dbdict as it stands, mention the original data-dict.yaml
      project only as lineage with the decision to move away noted;
      every factual claim re-verified against the repo (crate list, CLI
      subcommands, module layout, fixture dirs, schema files, Problem
      fields); the upstream comment policy ("default to no comment")
      contradicted the fork's training-wheels convention — replaced
      with a pointer to the repo-root CLAUDE.md; stale nanoparquet
      instruction dropped (download-examples.R uses no parquet); the
      original single-bullet `site/` fix was subsumed by this rewrite
- [x] README.md: no change — the fork-credit note (line 10) survived
      the sweep
- **verify:** `grep -rni "tidyverse" . --exclude-dir=target
  --exclude-dir=.git --exclude-dir=.claude-work` returns only
  deliberate attribution — README credit, root CLAUDE.md fork note,
  LICENSE copyright, and site/index.md's Lineage section (the last two
  weren't named in the original wording but are attribution by design,
  aligned with the goal's "attribution stays" scope); `cargo test
  --workspace` still green (290); quarto render skipped (quarto not
  installed — noted, best-effort per goal) — PASSED
