# fork branding

## problem

dbdict is a deliberate fork of tidyverse `data-dict`, but the upstream
branding still leaks everywhere a user (or a diagnostic) points outward:

- `LEARN_MORE_URL` in crates/dbdict/src/validate_spec.rs is
  `http://data-dict.tidyverse.org/` — S09's fix-it suggests pointing new
  dictionaries at upstream's site
- site/CNAME claims `data-dict.tidyverse.org`, a domain we don't own —
  publishing the site as-is would be wrong (and would fail)
- site/_quarto.yml carries upstream's site-url, repo-url, GitHub nav
  link, and their Plausible analytics script (our page views would land
  in their dashboard)
- site/index.md describes upstream `data-dict.yaml` in tidyverse voice —
  parquet-first, upstream examples — not the rich DuckDB-native format
  that is this fork's point
- ~10 test fixtures and doc examples embed the upstream URL as
  `$learn_more`

this is the last item blocking site publishing, and its decisions (name,
URL) leak into everything written afterwards — fixtures, docs, future
codegen headers — so it goes before the next generator session.

> decision (user, 2026-07-07): canonical `$learn_more` URL is the GitHub
> repo, https://github.com/pjc-wspace/dbdict — always valid, no site
> publishing required, can switch to a Pages URL later.

> decision (user, 2026-07-07): rebrand the site but publish later —
> delete CNAME, drop the Plausible script, point site-url/repo-url at
> pjc-wspace/dbdict. actual publishing is a separate decision.

> decision (user, 2026-07-07): index.md gets a dbdict-first rewrite —
> lead with the rich DuckDB-native dbdict.yaml format and the fork
> rationale; keep a short lineage note and upstream concepts that still
> apply.

> decision (user, 2026-07-07): sweep all fixtures/examples to the new
> URL — consistent branding everywhere; S09 only checks key presence so
> behaviour is unchanged, snapshots re-reviewed where they capture the
> URL.

## success criteria

- no occurrence of `data-dict.tidyverse.org` or
  `github.com/tidyverse/data-dict` anywhere in the repo except (a) the
  README's fork-credit note and (b) historical .claude-work/ records —
  verified by grep
- `LEARN_MORE_URL` = `https://github.com/pjc-wspace/dbdict`; S09's
  fix-it replacement renders the new URL (snapshot/test updated)
- site/CNAME deleted; _quarto.yml has no analytics script, and its
  site-url/repo-url/nav point at pjc-wspace/dbdict
- site/index.md reads as dbdict's front page: rich format first, fork
  lineage acknowledged, legacy path mentioned; examples links only to
  examples that still exist and validate
- upstream fork credit (README) preserved — attribution stays
- `cargo test --workspace` green; clippy + rustfmt clean; site still
  builds if quarto is available (best-effort check)

## scope

- in:
  - `LEARN_MORE_URL` constant + every fixture/test/doc embedding the old
    URL
  - site/CNAME (delete), site/_quarto.yml (rebrand), site/index.md
    (rewrite)
  - a grep-based "no upstream branding" verification step
- out:
  - actually enabling GitHub Pages / publishing — separate decision
  - renaming files or the `data-dict.yaml` legacy filename — the legacy
    path keeps its name by design
  - site/examples content beyond fixing dead references — example
    curation is its own session if wanted
  - upstream attribution removal — credit stays in README (MIT terms)

## constraints

- S09 checks only that `$learn_more` is present — no behaviour change
  from the URL sweep; snapshot diffs are presentation-only
- legacy (0.1.0) path untouched behaviourally
- site/spec.md and site/validation.md content changes limited to URL
  swaps — their substance was rewritten in earlier sessions
- maintainer is learning Rust — training-wheels comments where code is
  touched (the constant + any test updates)
