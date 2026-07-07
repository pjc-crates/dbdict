---
created: 2026-07-07T13:57:13+12:00
title: fork rebrand surface and inherited process files
tags: [workflow, design, gotcha]
source: /ws done
---

## rebrand surface extends beyond docs
Upstream branding hid in a fix-it constant (`LEARN_MORE_URL` — the S09 suggestion would have pointed new users at upstream's site), an analytics script (page views would land in upstream's Plausible dashboard), a CNAME claiming their domain, and an agent-instructions file. When rebranding a fork, grep for the domain, the org, and the bare project name separately — each catches references the others miss.

## forked agent-instruction files can contradict the fork's own conventions
Upstream's `.claude/claude.md` comment policy ("default to no comment") directly opposed the fork's training-wheels rule in the root CLAUDE.md. A fork inherits *process* files too, and they drift silently until reconciled — resolve by keeping one canonical policy and pointing at it rather than restating rival versions.
