---
title: "Claude Code: auto-loading behavior/commands at interactive session start (mechanisms other than SessionStart hook)"
date: 2026-07-10
sources:
  - https://code.claude.com/docs/en/cli-reference
  - https://code.claude.com/docs/en/skills
  - https://code.claude.com/docs/en/hooks
  - https://code.claude.com/docs/en/settings
  - https://code.claude.com/docs/en/output-styles
  - https://code.claude.com/docs/en/statusline
  - https://code.claude.com/docs/en/remote-control
  - https://code.claude.com/docs/en/memory
  - https://code.claude.com/docs/en/commands
  - https://code.claude.com/docs/llms.txt
  - https://github.com/anthropics/claude-code/issues/2735
  - https://github.com/anthropics/claude-code/issues/10282
  - https://github.com/anthropics/claude-code/issues/12357
---

## summary

Claude Code has **no documented, built-in mechanism to auto-invoke a slash
command or "mode" at the start of an interactive session**, other than the
`SessionStart` hook — and even the `SessionStart` hook cannot invoke a slash
command; it can only inject inert context text (`additionalContext`) or, in
**non-interactive `-p` mode only**, seed a first user message
(`initialUserMessage`). Three separate GitHub feature requests asking for
exactly this capability (auto-run `/command` on startup) are all **closed,
one explicitly "not planned"**, and a maintainer-adjacent comment on one of
them states plainly: "Hooks cannot run slash commands."

`--append-system-prompt` and `--system-prompt` are real, documented flags
that work in interactive mode, but they modify the **system prompt text**,
not the conversation — they cannot make Claude behave as if you typed
`/some-command`, and the docs themselves note `--append-system-prompt` "must
be passed every invocation, so it's better suited to scripts and automation
than interactive use" (from the CLAUDE.md troubleshooting page). Custom
slash commands (`.claude/commands/*.md`, now unified with `.claude/skills/`)
are purely on-demand — invoked by typing `/name` or by Claude's own
judgment — with no config to fire one automatically at startup.
`companyAnnouncements` in `settings.json` is the only settings key that
displays something automatically at startup, and it's a static banner
message, not an action. `/remote-control` (alias `/rc`) **is** a real
built-in slash command, and it does have an "enable for all sessions"
toggle — but that's a Remote Control auto-connect switch, not a general
startup-automation feature, and it's set via `/config`, not by
auto-invoking the command as text.

## detailed findings

### 1. `--append-system-prompt` / `--system-prompt`

Per the CLI reference (`https://code.claude.com/docs/en/cli-reference`):

- `--system-prompt` — "Replace the entire system prompt with custom text."
  Works in both interactive and non-interactive modes.
- `--system-prompt-file` — same, but loaded from a file. Mutually exclusive
  with `--system-prompt`.
- `--append-system-prompt` — "Append custom text to the end of the default
  system prompt (does not replace it)." Works in both interactive and
  non-interactive modes. As of Claude Code v1.0.51 this flag works in
  interactive mode, not just print mode (per secondary source
  `aiengineerguide.com`, consistent with the official docs' "works in both
  modes" statement).
- `--append-system-prompt-file` — same, loaded from a file.
- `--append-subagent-system-prompt` — appends to every subagent's system
  prompt (including nested subagents); **non-interactive (`-p`) mode only**,
  requires v2.1.205+.

**Can these simulate "a slash command was invoked at startup"?** No — by
design they only affect the system prompt (Claude's standing instructions),
not the conversation transcript or tool-invocation state. The official
output-styles doc's comparison table is explicit about the distinction:

> "`--append-system-prompt` | Appends to the system prompt without removing
> anything | You want a one-off addition for a single invocation"
> — https://code.claude.com/docs/en/output-styles

And the memory (CLAUDE.md) troubleshooting page states directly that this
flag is not meant for routine interactive startup automation:

> "For instructions you want at the system prompt level, use
> `--append-system-prompt`. This must be passed every invocation, so it's
> better suited to scripts and automation than interactive use."
> — https://code.claude.com/docs/en/memory

So the practical pattern people use is a shell alias/wrapper script that
always passes `--append-system-prompt "..."`, not a persisted "run this at
startup" config — there is no settings.json key that auto-applies an append
prompt without the flag being passed on every invocation.

Related startup-adjacent flags found in the CLI reference (not
system-prompt related, but worth noting since they run things *before* a
session):
- `--init` — "Run Setup hooks with the `init` matcher before the session."
  Print mode only.
- `--init-only` — "Run Setup and SessionStart hooks, then exit without
  starting a conversation." Both modes.
- `--maintenance` — "Run Setup hooks with the `maintenance` matcher before
  the session." Print mode only.

These all route through the hook system (`Setup` / `SessionStart` hooks),
not a slash-command or system-prompt mechanism.

### 2. Custom slash commands: `.claude/commands/*.md`

Per `https://code.claude.com/docs/en/skills` (the current canonical page —
the old `/en/slash-commands` docs URL now redirects here):

> "Custom commands have been merged into skills. A file at
> `.claude/commands/deploy.md` and a skill at `.claude/skills/deploy/SKILL.md`
> both create `/deploy` and work the same way. Your existing
> `.claude/commands/` files keep working. Skills add optional features: a
> directory for supporting files, frontmatter to control whether you or
> Claude invokes them, and the ability for Claude to load them automatically
> when relevant."

Definition mechanics:
- A file under `.claude/commands/` (or `.claude/skills/<name>/SKILL.md`)
  becomes command `/<filename-without-extension>` or `/<directory-name>`
  respectively.
- Frontmatter fields include `name`, `description`, `when_to_use`,
  `argument-hint`, `arguments`, `disable-model-invocation`,
  `user-invocable`, `allowed-tools`, `disallowed-tools`, `model`, `effort`,
  `context` (`fork` to run as subagent), `agent`, `hooks`, `paths`, `shell`.
- `$ARGUMENTS`, `$ARGUMENTS[N]`, `$N`, and named `$name` substitutions are
  supported for passing arguments.
- Locations: personal (`~/.claude/skills/`), project (`.claude/skills/`),
  plugin (`<plugin>/skills/`), enterprise (managed settings dir). Precedence:
  enterprise > personal > project; any level overrides a same-named bundled
  skill.

**No auto-invoke-at-startup mechanism exists.** Invocation is always either
(a) you typing `/name`, or (b) Claude deciding to load it automatically
*during a turn* based on its `description` matching your prompt (unless
`disable-model-invocation: true` is set, which restricts it to manual-only).
There is no frontmatter field, settings key, or CLI flag that fires a
command the instant a session opens, before any user input.

This gap is confirmed by real user demand — see the GitHub issues in the
"community/GitHub evidence" section below.

### 3. Output styles / statusline / settings.json keys

**Output styles** (`https://code.claude.com/docs/en/output-styles`):
- Modify the system prompt (role, tone, format). Four built-in styles:
  Default, Proactive, Explanatory, Learning.
- Selected via `/config` → **Output style**, saved to
  `.claude/settings.local.json`, or set directly:
  ```json
  { "outputStyle": "Explanatory" }
  ```
- Explicitly **read once at session start**: "Output style is part of the
  system prompt, which Claude Code reads once at session start. Changes
  take effect after `/clear` or a new session."
- This *is* a way to make Claude behave in a particular "mode" from the
  first response of a session, but it must be set in a settings file ahead
  of time — it does not execute an action, it only changes response
  style/tone, and it's config, not an auto-invoked command.

**statusLine** (`https://code.claude.com/docs/en/statusline`):
- "The status line is a customizable bar at the bottom of Claude Code that
  runs any shell script you configure. It receives JSON session data on
  stdin and displays whatever your script prints."
- Configured via `settings.json`:
  ```json
  { "statusLine": { "type": "command", "command": "~/.claude/statusline.sh", "padding": 0 } }
  ```
- This runs continuously/repeatedly (rendered on a tick, receiving live
  session JSON), not as a one-shot startup action — it's a persistent
  display, not an auto-run-once mechanism.

**settings.json keys relevant to "run/show something at startup"**
(`https://code.claude.com/docs/en/settings`):
- `companyAnnouncements` — "Announcement to display to users at startup. If
  multiple announcements are provided, they will be cycled through at
  random." This is the **only** settings key confirmed to auto-display
  something specifically "at startup," and it's a static text banner, not
  an action or command.
- `claudeMd` — "(Managed settings only) CLAUDE.md-style instructions
  injected as organization-managed memory. Only honored when set in managed
  or policy settings and ignored in user, project, and local settings."
  Still just injected text/context, not an invoked command.
- `hooks` — "Configure custom commands to run at lifecycle events." This is
  the general hook config surface (see §4).
- `agent` — "Run the main thread as a named subagent... Applies that
  subagent's system prompt, tool restrictions, and model." This changes
  *what Claude is* for the whole session, closer to a "mode" than any other
  setting, but it's a persona/subagent swap, not a slash-command
  invocation.
- Confirmed **no** keys named/related to `startup`, `onStart`, `autorun`,
  `defaultPrompt`, `sessionInit`, `firstMessage`, or `initialPrompt` exist
  in the settings.json schema as documented.
- Confirmed restart-only keys: "A few keys are read once at session start
  and apply on the next restart instead: `model` ... `outputStyle`: part of
  the system prompt, which is rebuilt on `/clear` or restart."

### 4. `SessionStart` hook capabilities (for context, since the ask was "other than SessionStart")

Documented at `https://code.claude.com/docs/en/hooks`. Full hook-event list
includes `Setup`, `SessionStart`, `UserPromptSubmit`, `UserPromptExpansion`,
`Stop`, `StopFailure`, `PreToolUse`, `PermissionRequest`,
`PermissionDenied`, `PostToolUse`, `PostToolUseFailure`, `PostToolBatch`,
`SubagentStart`, `SubagentStop`, `TaskCreated`, `TaskCompleted`,
`Notification`, `MessageDisplay`, `TeammateIdle`, `PreCompact`,
`PostCompact`, `ConfigChange`, `CwdChanged`, `FileChanged`,
`InstructionsLoaded`, `WorktreeCreate`, `WorktreeRemove`, `Elicitation`,
`ElicitationResult`, `SessionEnd`.

`SessionStart` is the one that "fires when a session begins or resumes,"
with matchers `startup`, `resume`, `clear`, `compact`, and it can return
these JSON output fields (quoted verbatim from
`https://code.claude.com/docs/en/hooks`):

| Field | Description |
|---|---|
| `additionalContext` | "String added to Claude's context at the start of the conversation, before the first prompt." |
| `initialUserMessage` | "String used as the first user message of the session. **Applies in non-interactive mode with the `-p` flag**, where it becomes the first turn even if no prompt is provided. If a prompt is provided, it follows as the next turn. Unlike `additionalContext`, which attaches to an existing turn, this creates the turn." |
| `sessionTitle` | "Sets the session title, with the same effect as `/rename`." Applies only on `source: "startup"` or `"resume"`. |
| `watchPaths` | Array of absolute paths to watch for `FileChanged` events. |
| `reloadSkills` | Boolean — re-scans skill/command directories after the hook completes. |

**Key finding: even `SessionStart`'s most command-like field,
`initialUserMessage`, is explicitly documented as applying only in
non-interactive `-p` mode** — not in an interactive session that stays
open. In interactive mode, `SessionStart` can only add passive context
(`additionalContext`); it cannot inject a message or slash command that
Claude treats as user input. This directly explains why the GitHub feature
requests below exist and remain unimplemented.

### 5. `/remote-control` — is it a real built-in command?

Yes. Fully documented at `https://code.claude.com/docs/en/remote-control`.
It is a genuine, current, built-in Claude Code feature (not a
plugin/community command), in **research preview**, available on Pro, Max,
Team, and Enterprise plans (not API keys).

Three ways to start it:
1. `claude remote-control` — server mode, standalone process.
2. `claude --remote-control` (or `--rc`) — starts a normal interactive
   session with Remote Control enabled from the first prompt.
3. From inside an existing session: `/remote-control` (alias `/rc`) —
   "starts a Remote Control session that carries over your current
   conversation history."

It connects the local terminal session to `claude.ai/code` or the Claude
mobile app so you can steer the same local session from another device;
your filesystem, MCP servers, and tools stay local. It requires
`claude.ai` OAuth login (not API-key auth) and is disabled when
`ANTHROPIC_BASE_URL` points somewhere other than `api.anthropic.com`.

**Auto-enabling for every session** — this is the one place in the docs
where something resembling "startup automation for a command" genuinely
exists, but it's a dedicated feature switch, not a general mechanism:

> "Remote Control only activates when you explicitly run `claude
> remote-control`, `claude --remote-control`, or `/remote-control`, unless
> auto-connect is turned on. To enable it automatically for every
> interactive session, run `/config` inside Claude Code and set **Enable
> Remote Control for all sessions** to `true`." — remote-control docs

This setting is also exposed in the Desktop app (**Settings → Claude Code →
Enable remote control by default**) and in the VS Code extension's command
menu. It is a boolean toggle specific to the Remote Control feature, saved
via `/config`, not a general "run any slash command at startup" facility.

## community / GitHub evidence that no general mechanism exists

Three separate feature requests on `anthropics/claude-code` ask for exactly
the capability this research was scoped to find, and none exist as shipped
features:

- **[Issue #2735](https://github.com/anthropics/claude-code/issues/2735)**
  — "Auto-load slash commands on startup." Proposed a `--slash-command
  /alex` flag or a CLAUDE.md/config-based default persona. **Closed as not
  planned** (labels: `area:core`, `area:tui`, `enhancement`, `autoclose`).
  Reporter's attempted workarounds all failed: shell alias with a
  non-existent flag, relying on CLAUDE.md (read as reference only, doesn't
  "activate" a command), and piping `echo '/alex' | claude` (breaks
  Claude Code's raw-terminal-mode requirement).
- **[Issue #10282](https://github.com/anthropics/claude-code/issues/10282)**
  — "Auto-execute slash commands on session start," explicitly asking to
  extend `SessionStart` hooks to support a `slash-command` hook type (or a
  new `sessionInit` config), motivated by needing to run `/restore-memory`
  for MCP context-restoration workflows every session. **Closed**, priority
  labeled "Critical," no shipped workaround documented in the visible
  discussion.
- **[Issue #12357](https://github.com/anthropics/claude-code/issues/12357)**
  — "Ability to run slash command in CLAUDE.md," reporting that Claude
  "doesn't consistently follow instructions in CLAUDE.md files and cannot
  automatically execute slash commands," wanting `SlashCommand(/setup-claude)`
  to run automatically every new session and after `/clear`. **Closed**.
  The thread's stated limitation: **"Hooks cannot run slash commands"** —
  i.e., confirmation from within the issue discussion that the hook system
  (the sanctioned extensibility point) is architecturally unable to invoke
  a `/command`, which is consistent with the `initialUserMessage`
  interactive-mode restriction found in the official docs (§4).

Taken together, this is strong evidence (docs + issue tracker, not
speculation) that as of the current Claude Code version, **there is no
supported way to make an interactive session auto-run a slash command at
startup** — the closest things are: (a) `SessionStart` hook's
`additionalContext` (passive text injection only), (b) CLAUDE.md content
(also passive, and explicitly "not enforced configuration" — Claude may or
may not act on it), (c) an `outputStyle` set in advance (changes tone/role,
not an invoked action), and (d) the Remote-Control-specific auto-connect
toggle, which only applies to that one feature.

## sources

### https://code.claude.com/docs/en/cli-reference
Official CLI reference. Documents `--system-prompt`, `--system-prompt-file`,
`--append-system-prompt`, `--append-system-prompt-file` (all work in both
interactive and non-interactive modes) and `--append-subagent-system-prompt`
(non-interactive `-p` only, v2.1.205+). Also documents `--init` (print-mode
only, runs Setup hooks with `init` matcher), `--init-only` (both modes, runs
Setup + SessionStart hooks then exits without a conversation), and
`--maintenance` (print-mode only, runs Setup hooks with `maintenance`
matcher). No flag exists for invoking a slash command at startup. Verified
live (curl HTTP 200).

### https://code.claude.com/docs/en/skills
Canonical current doc for both skills and legacy `.claude/commands/`
(the old `/en/slash-commands` URL redirects/aliases here). Explains that
custom commands were "merged into skills," that `.claude/commands/*.md`
still works, full frontmatter reference
(`name`, `description`, `when_to_use`, `argument-hint`, `arguments`,
`disable-model-invocation`, `user-invocable`, `allowed-tools`,
`disallowed-tools`, `model`, `effort`, `context`, `agent`, `hooks`, `paths`,
`shell`), string substitutions (`$ARGUMENTS`, `$N`, `$name`,
`${CLAUDE_SESSION_ID}`, `${CLAUDE_EFFORT}`, `${CLAUDE_SKILL_DIR}`,
`${CLAUDE_PROJECT_DIR}`), storage locations/precedence (enterprise >
personal > project, plugin namespaced), live change detection, and
`skillOverrides` settings-based visibility control. No startup
auto-invocation mechanism documented anywhere on this page.

### https://code.claude.com/docs/en/hooks
Official hooks documentation. Enumerates the full hook-event list
(Setup, SessionStart, UserPromptSubmit, UserPromptExpansion, Stop,
StopFailure, PreToolUse, PermissionRequest, PermissionDenied, PostToolUse,
PostToolUseFailure, PostToolBatch, SubagentStart, SubagentStop,
TaskCreated, TaskCompleted, Notification, MessageDisplay, TeammateIdle,
PreCompact, PostCompact, ConfigChange, CwdChanged, FileChanged,
InstructionsLoaded, WorktreeCreate, WorktreeRemove, Elicitation,
ElicitationResult, SessionEnd). Gives the exact SessionStart output-field
table: `additionalContext`, `initialUserMessage` (explicitly "Applies in
non-interactive mode with the `-p` flag"), `sessionTitle`, `watchPaths`,
`reloadSkills`. This is the critical page confirming that even the
SessionStart hook cannot inject a simulated user command in an interactive
session. Verified live (curl HTTP 200); note the sibling URL
`/en/hooks-reference` returns HTTP 404 and should not be used.

### https://code.claude.com/docs/en/settings
Official settings.json reference. Confirmed exact quotes for
`companyAnnouncements` ("Announcement to display to users at startup...
cycled through at random"), `claudeMd` (managed-settings-only, injected as
org-managed memory), `hooks` ("Configure custom commands to run at
lifecycle events"), `agent` (run main thread as a named subagent). Confirmed
`model` and `outputStyle` are "read once at session start" and require
`/clear` or restart to change. Confirmed search found **no** settings key
named/related to `startup`, `onStart`, `autorun`, `defaultPrompt`,
`sessionInit`, `firstMessage`, or `initialPrompt`. Verified live (curl HTTP
200).

### https://code.claude.com/docs/en/output-styles
Official output-styles documentation. Four built-in styles (Default,
Proactive, Explanatory, Learning); custom styles are Markdown files with
frontmatter (`name`, `description`, `keep-coding-instructions`,
`force-for-plugin`) stored at `~/.claude/output-styles`,
`.claude/output-styles`, or a plugin's `output-styles/` dir. Selected via
`/config` or the `outputStyle` settings key; explicitly "read once at
session start," takes effect after `/clear` or a new session. Contains the
comparison table that classifies `--append-system-prompt` as for "a one-off
addition for a single invocation" versus CLAUDE.md/output styles/agents/
skills for persistent behavior — directly useful for answering whether
`--append-system-prompt` can simulate a startup mode switch (it can affect
tone/behavior but not invoke an action).

### https://code.claude.com/docs/en/statusline
Official statusline documentation. Confirms the status line "runs any
shell script you configure," receiving JSON session data on stdin,
rendered continuously (not a one-shot startup action) in its own row above
the footer. Configured via the `statusLine` settings.json key
(`{"statusLine": {"type": "command", "command": "...", "padding": 0}}`).
Relevant as a "runs something related to session state" mechanism, but not
an auto-invoked command/mode switch.

### https://code.claude.com/docs/en/remote-control
Official Remote Control documentation — the definitive source confirming
`/remote-control` (alias `/rc`) is a real, current, built-in slash command,
in research preview, on Pro/Max/Team/Enterprise (not API-key auth). Three
invocation modes: `claude remote-control` (server mode), `claude
--remote-control`/`--rc` (interactive session with RC enabled from start),
and `/remote-control` from within an existing session. Documents the
"Enable Remote Control for all sessions" toggle set via `/config`
(also available in Desktop app settings and VS Code extension) — the one
genuine "enable a command automatically for every interactive session"
switch found in the entire research, but scoped only to this feature.
Also covers Trusted Devices (beta, Team/Enterprise), mobile push
notifications, limitations (one remote session per process outside server
mode, local process must keep running, disconnects on Ultraplan start),
and a comparison table of Dispatch/Remote Control/Channels/Slack/Scheduled
tasks as different "work when not at your terminal" options.

### https://code.claude.com/docs/en/memory
Official CLAUDE.md / auto-memory documentation. Confirms "CLAUDE.md content
is delivered as a user message after the system prompt, not as part of the
system prompt itself" and that Claude "reads it and tries to follow it, but
there's no guarantee of strict compliance" — i.e., CLAUDE.md cannot reliably
force an action like invoking a slash command; it's advisory context, not
enforced. Explicitly recommends hooks for anything that "must run at a
specific point" and states `--append-system-prompt` "must be passed every
invocation, so it's better suited to scripts and automation than
interactive use" — directly relevant confirmation for question 1. Also
covers CLAUDE.md file discovery/load order, `.claude/rules/` path-scoped
rules, auto memory (`MEMORY.md`, `autoMemoryEnabled`,
`autoMemoryDirectory`), and managed/org-wide `claudeMd` settings key.

### https://code.claude.com/docs/en/commands
Commands reference page. Confirmed `/remote-control` (alias `/rc`) is
listed among built-in commands under an "Integration" category alongside
`/desktop`, `/remote-env`, `/teleport`. Lists other built-in commands
(`/clear`, `/resume`, `/branch`, `/fork`, `/background`, `/cd`, `/exit`,
`/model`, `/effort`, `/code-review`, `/simplify`, `/security-review`,
`/run`, `/debug`, `/verify`, `/loop`, `/goal`, `/init`, `/memory`,
`/permissions`, `/mcp`, `/hooks`, `/context`, `/compact`, `/plan`, `/config`,
`/help`, `/skills`, `/plugin`, `/doctor`, etc.). Did not contain the
detailed skill-frontmatter schema (that content lives on `/en/skills`
instead) or any startup-auto-invocation mechanism.

### https://code.claude.com/docs/llms.txt
Documentation site index used to resolve correct canonical page paths
(confirmed `/en/hooks-reference` and `/en/slash-commands` are stale/broken
paths; current pages are `/en/hooks` and `/en/skills` respectively).

### https://github.com/anthropics/claude-code/issues/2735
"Feature Request: Auto-load slash commands on startup." Closed as not
planned (labels: area:core, area:tui, enhancement, autoclose). Reporter
wanted a default persona command (`/alex`) to run automatically at
startup; proposed a `--slash-command` flag, CLAUDE.md-based activation, or
a config default. All attempted workarounds (shell alias with nonexistent
flag, CLAUDE.md as reference only, `echo '/alex' | claude` piping) failed
because Claude Code requires an interactive raw terminal.

### https://github.com/anthropics/claude-code/issues/10282
"[FEATURE] Auto-execute slash commands on session start." Closed, labeled
"Critical" priority. Proposed extending `SessionStart` hooks with a new
`"type": "slash-command"` hook entry, or a new `sessionInit.commands`
settings array, motivated by needing to run `/restore-memory` for MCP
context-restoration (e.g. `mcp-memory-keeper`) every session. Comment
thread raised security considerations (explicit opt-in, confirmation
prompt, enable/disable toggle, restricting to "safe" commands) but no
workaround or shipped feature is documented in the visible discussion.

### https://github.com/anthropics/claude-code/issues/12357
"[FEATURE] Ability to run slash command in CLAUDE.md." Closed. Reporter
noted Claude "doesn't consistently follow instructions in CLAUDE.md files
and cannot automatically execute slash commands," wanted
`SlashCommand(/setup-claude)`-style directives in CLAUDE.md to run
automatically every session and after `/clear`. The discussion states the
underlying limitation plainly: **"Hooks cannot run slash commands"** — the
clearest community-level confirmation that the sanctioned hook
extensibility point is architecturally incapable of this, consistent with
the official docs' `initialUserMessage` interactive-mode restriction.
