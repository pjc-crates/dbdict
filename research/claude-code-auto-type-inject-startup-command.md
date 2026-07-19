---
title: "Auto-typing / injecting a command into an interactive terminal CLI session at startup (staying open) — Claude Code and general techniques"
date: 2026-07-10
sources:
  - https://code.claude.com/docs/en/cli-reference
  - https://code.claude.com/docs/en/interactive-mode
  - https://code.claude.com/docs/en/headless
  - https://code.claude.com/docs/en/terminal-config
  - https://github.com/anthropics/claude-code/issues/6009
  - https://github.com/anthropics/claude-code/issues/3180
  - https://github.com/anthropics/claude-code/issues/3844
  - https://github.com/anthropics/claude-code/issues/15553
  - https://github.com/anthropics/claude-code/issues/12507
  - https://github.com/anthropics/claude-code-action/issues/523
  - https://dev.to/ikramar/i-wrapped-claude-code-in-a-zsh-function-heres-every-decision-i-almost-got-wrong-2ofc
  - https://github.com/mushfoo/claude-fish
  - https://www.hsablonniere.com/dotfiles-claude-code-my-tiny-config-workshop--95d5fr/
  - https://gist.github.com/GGPrompts/800f2c67d96bceab836c0090b71488ef
  - https://hboon.com/using-tmux-with-claude-code/
  - https://crossaitools.com/skills/obra/superpowers-lab/using-tmux-for-interactive-commands
  - https://www.devas.life/how-to-run-claude-code-in-a-tmux-popup-window-with-persistent-sessions/
  - https://raw.githubusercontent.com/smtg-ai/claude-squad/main/README.md
  - https://raw.githubusercontent.com/claude-yolo/claude-yolo/main/README.md
---

## summary

**Officially documented (angle 1):** `claude "some prompt"` is a real, documented
positional argument that **starts interactive mode with that prompt already
submitted**, then stays open for follow-up turns. This is distinct from
`claude -p "some prompt"` (`--print`), which is **non-interactive** and exits
after one response. The CLI reference table states plainly:
`claude "query"` → "Start interactive session with initial prompt", vs.
`--print`, `-p` → "Print response without interactive mode." Slash commands
(user-invoked skills/custom commands) embedded in the prompt string are
expanded by Claude Code before running — confirmed for `-p` mode by the docs
("include `/skill-name` in the prompt string and Claude Code expands it
before running"), with the caveat that terminal-only built-ins like `/login`
are unavailable outside the interactive TUI. No official doc statement
explicitly confirms slash-command expansion for the *positional-arg-into-
interactive-mode* path specifically, but the mechanism (prompt string →
same input pipeline) is the same one described for `-p`, so it is reasonable
to expect it works there too — flagged below as inference, not a quote.

**Known gotchas (angle 1, from GitHub issues):**
- `claude -c "prompt"` (continue + inline prompt) is **broken**: the TUI
  resumes the previous conversation but silently ignores the new prompt,
  leaving the session idle (issue #3180, Claude Code 1.0.44, closed).
- A prompt argument starting with `-` gets misparsed as a CLI flag rather
  than prompt text, e.g. `-1 + 2 = ?` → `error: unknown option '-1 + 2 = ?'`.
  Fix is the POSIX `--` separator before the prompt (issue #3844).
- Anthropic explicitly declined (**"closed as not planned"**) a feature
  request to pre-populate the interactive prompt buffer from piped stdin
  (`cat file | claude` opening the TUI with the file content sitting in the
  input box, editable before submit) — issue #6009. Piping today only works
  with `-p` (non-interactive, exits).

**Shell wrappers (angle 2):** community fish/zsh wrapper *functions*
(not aliases) are the common pattern, chosen specifically because a shell
function inherits the current process's tty cleanly — necessary since Claude
Code's TUI needs direct terminal access. No wrapper found that pre-seeds a
*slash command* specifically; wrappers instead branch on subcommands to
prepend flags (`--append-system-prompt`, `--permission-mode`, worktree
setup) before handing off `claude "$@"` to the real binary.

**Terminal automation (angle 3):** `tmux send-keys` is the dominant
community technique and it does work for submitting text into a running
Claude Code TUI — because tmux writes into a real pseudo-terminal, which
Claude Code's Ink-based input component treats as physical typing. The
consistent community pattern across three independent tools (a "pmux"
slash-command gist, `claude-yolo`, and an Anthropic-adjacent "superpowers"
tmux skill) is: send the literal text in `-l` (literal) mode, **sleep
briefly** (~0.3s or more, sometimes with `capture-pane` polling instead of a
fixed sleep), then send `Enter`/`C-m` **as a separate `send-keys` call**.
Bundling text and Enter in one call, or submitting before the TUI has
finished rendering the prompt box, is the most commonly cited failure mode
("race condition" / dropped keystrokes). By contrast, **`expect`-style
programmatic writes that don't go through a real pty** (VS Code's
`terminal.sendText()`/`sendSequence`, macOS AppleScript `keystroke`) are
reported to **fail to submit**: an open feature request (#15553) documents
that Claude Code's Ink `ink-text-input` component distinguishes a physical
Enter keypress (triggers submit) from a programmatic `\r`/`\n` byte written
via those APIs (treated as a literal newline, no submit) — this is the load-
bearing gotcha for anyone trying non-pty automation. `screen -X stuff` is
the `screen` equivalent of `tmux send-keys` (writes into the pty), so the
same reasoning should apply, but no source discusses it specifically for
Claude Code — flagged as inference below, not sourced.

**CLI flags for angle 4:** there is **no dedicated flag** that "pre-seeds
the first message but forces/guarantees interactive mode" beyond the
positional argument itself — the positional argument *is* the documented
mechanism. `--continue`/`-c` combined with a prompt is broken per above.
`--resume`/`-r` with a session name plus a trailing prompt is documented to
resume and stay interactive. No flag pre-populates the editable input buffer
(that was the rejected #6009 request).

---

## detailed findings

### 1. Positional prompt argument: does it stay open or exit?

From the official CLI reference table (quoted verbatim):

> `claude "query"` — Start interactive session with initial prompt — example: `claude "explain this project"`

> `--print`, `-p` — Print response without interactive mode (see Agent SDK documentation for programmatic usage details) — example: `claude -p "query"`

Source: https://code.claude.com/docs/en/cli-reference

This directly answers the question: **the positional argument alone keeps the
session interactive**; it is `-p`/`--print` that causes the one-shot,
exit-after-response behavior. Multiple secondary sources (eesel AI's CLI
reference summary, blakecrosley.com's guide, community WebSearch synthesis)
independently paraphrase the same distinction, so this is well corroborated,
not a single-source claim.

**Slash commands as the positional/prompt-string argument.** The
`/en/headless` doc (covering `-p` mode) states verbatim:

> "User-invoked skills and custom commands work in `-p` mode: include
> `/skill-name` in the prompt string and Claude Code expands it before
> running. Built-in commands that only run in the terminal interface, such
> as `/login`, aren't available in `-p` mode."

Source: https://code.claude.com/docs/en/headless

This confirms slash commands *do* get expanded when embedded in a prompt
string, at least for `-p`. `Inferred:` since the positional-argument-into-
interactive-mode path feeds the same prompt-processing pipeline as the first
turn of any interactive session (where typing `/command` is the documented
way to invoke it — see `/en/interactive-mode`, "Type `/` in Claude Code to
see all available commands"), it is very likely `claude "/some-command"`
also expands the command as the first turn. No page states this outright for
the *interactive* positional-arg case, so treat as inference, not fact.

A related, but distinct, real bug exists in **claude-code-action** (the
GitHub Action, which wraps the SDK's non-interactive/agentic mode, not the
interactive TUI): passing a slash command as the Action's `prompt:` input
causes execution to halt immediately with zero tokens consumed and an empty
result (issue #523, https://github.com/anthropics/claude-code-action/issues/523).
This is a different code path from the CLI's own `-p`/positional-arg
handling described above, but is worth knowing if anyone is trying to
reproduce "auto-inject a slash command at startup" inside a GitHub Action
rather than a real terminal.

### 2. Known bugs/gotchas around the positional prompt argument

- **`-c`/`--continue` + inline prompt is silently ignored.** Issue #3180
  (https://github.com/anthropics/claude-code/issues/3180): `claude -c "Can
  you see this"` resumes the previous TUI session and displays history, but
  the new prompt text is never processed — "the interactive TUI session
  resumes with the previous conversation visible, but the new prompt from
  the command line is not processed. The session sits idle waiting for
  manual user input instead." Reported against Claude Code 1.0.44,
  macOS 14.3, closed. **Workaround used by the CLI reference table itself**:
  `--resume`/`-r` with a session name *plus* a trailing query is documented
  separately as working and staying interactive (`claude -r "session-name"
  "query"`), so prefer `-r` over `-c` if you need "resume + inject text."

- **Prompt text starting with `-` is misparsed as a flag.** Issue #3844
  (https://github.com/anthropics/claude-code/issues/3844), reported against
  the SDK's `query()` wrapper but rooted in the same CLI arg parser: a
  prompt of `-1 + 2 = ?` produces `error: unknown option '-1 + 2 = ?'`.
  Fix/workaround discussed: insert the POSIX `--` end-of-options separator
  before the prompt so the parser stops looking for flags,
  e.g. `claude -- "-1 + 2 = ?"`. Relevant if your auto-injected command text
  could plausibly start with `-` (unlikely for slash commands, since those
  start with `/`, but worth knowing for prompt text in general).

- **Piping stdin to prime the interactive input box was explicitly
  rejected.** Issue #6009
  (https://github.com/anthropics/claude-code/issues/6009) asked for
  `git diff main | claude` (no `-p`) to open the TUI with the diff already
  sitting in the editable prompt box, cursor at the end, so the user could
  add instructions before submitting — closed **"not planned"**. Today,
  piping only works with `-p`, which is non-interactive and exits
  (`cat file | claude -p "query"`).

- **stdin can be silently consumed by shell-detection subshells on some
  systems (HPC/cluster interactive sessions), causing immediate EOF-exit.**
  Issue #12507 (https://github.com/anthropics/claude-code/issues/12507):
  strace showed Claude Code's shell-environment-detection subprocesses
  inheriting and draining the parent's stdin fd before the interactive
  prompt loop could read from it, so the session exits cleanly (code 0)
  instead of becoming interactive — even though `test -t 0` reports stdin
  as a valid tty. Regression noted between 2.0.45 → 2.0.54. Not directly
  about prompt injection, but relevant if you're scripting `claude` launch
  inside a wrapper that spawns subshells (e.g. an `expect` script or a CI
  runner) and see it exit immediately for no apparent reason — check
  whether something upstream of `claude` is eating stdin first.
  Non-interactive `-p` mode was reported to sidestep the bug entirely.

### 3. Shell aliases / wrapper functions

No wrapper was found in the wild that specifically pre-seeds a *slash
command*. The pattern instead is argument/flag injection, and there's
explicit, sourced reasoning for **function over alias**:

From a detailed zsh-wrapper writeup
(https://dev.to/ikramar/i-wrapped-claude-code-in-a-zsh-function-heres-every-decision-i-almost-got-wrong-2ofc):

> "A zsh function runs in the current shell. Inherits the tty cleanly. Can
> dispatch on subcommands."

The author explicitly frames this as necessary because "Claude Code wraps
an interactive process requiring direct terminal access for prompt
rendering." Gotchas the author called out:
- naming the wrapper `cc` shadowed `/usr/bin/cc` (the C compiler), which
  silently broke Rust build scripts invoking `cc` — renamed to avoid it.
- subcommand-style dispatch (`cco plan`, `cco safe`, `cco here`) rather than
  extra flags, specifically to avoid colliding with Claude Code's 50+ native
  flags — with an acknowledged edge case that `cco "plan my vacation"`
  incorrectly matches the `plan` subcommand (workaround: `cco run "plan my
  vacation"`).
- the `resume` subcommand breaks (TUI session picker fails to render) when
  piped through `tee` for logging.

A fish-specific wrapper, `mushfoo/claude-fish`
(https://github.com/mushfoo/claude-fish), routes between `claude` and
`claude-trace` based on flags (e.g. `-T`/`--with-tracing`) and advertises
"seamless argument passthrough" / "full compatibility," but its actual
function code wasn't visible in the fetched README — it does not appear to
inject an initial prompt, only route which binary runs. Install pattern per
its docs: drop files into `~/.config/fish/functions/` and
`~/.config/fish/completions/`.

One dotfiles writeup (hsablonniere.com,
https://www.hsablonniere.com/dotfiles-claude-code-my-tiny-config-workshop--95d5fr/)
covers fish keyboard shortcuts and alias→function conversions generally but
had **no** `claude`-specific wrapper code — not a useful source for this
angle despite ranking well in search.

Simple alias-based system-prompt injection also appears in the wild, e.g.
`alias cc="claude --permission-mode acceptEdits --append-system-prompt ..."`,
but per the same sourcing, aliases don't support subcommand branching, so
authors who need conditional logic move to functions.

### 4. Terminal-automation tricks: tmux send-keys, expect, screen -X stuff

**tmux send-keys is the documented, working approach; the timing pattern is
consistent across three independent sources.**

From the "pmux" Claude Code slash-command gist
(https://gist.github.com/GGPrompts/800f2c67d96bceab836c0090b71488ef), which
sends a crafted prompt into a running Claude Code pane:

```bash
tmux send-keys -t "$TARGET_PANE" -l "COMPLETE FINAL PROMPT"
sleep 0.3
tmux send-keys -t "$TARGET_PANE" C-m
```

- `-l` = literal mode, so special characters in the prompt aren't
  reinterpreted by tmux's key-name parser.
- The doc explicitly calls the 0.3s sleep "CRITICAL" — it "prevents submit
  before prompt loads (especially for long prompts)."
- `C-m` (carriage return) is sent as a **separate** `send-keys` invocation
  from the text itself.

The same two-step pattern (text, then delayed separate Enter) is echoed by
an Anthropic-ecosystem "superpowers" tmux skill
(https://crossaitools.com/skills/obra/superpowers-lab/using-tmux-for-interactive-commands):
"always send Enter as a separate argument" and "sleep briefly after
starting a session" — combining Enter with other input in one `send-keys`
call is called out as the main cause of dropped/misinterpreted keystrokes.
That skill also recommends **polling `capture-pane`** to confirm the target
screen state (i.e., the prompt box is actually rendered and idle) instead
of relying purely on a fixed sleep, for more reliable automation than a
guessed delay.

`claude-yolo` (https://github.com/claude-yolo/claude-yolo), which
auto-approves Claude Code's permission dialogs for unattended parallel
agents, uses the same `capture-pane` + `send-keys` combo defensively: it
polls the pane (every 0.3s by default) for permission-dialog markers
("Allow"/"Deny", or a `❯`-prefixed numbered option list plus tool-name/
"requires approval" keywords), then sends `Enter` via `tmux send-keys` to
confirm the pre-selected "Yes" option, or the option's number if "Yes"
isn't pre-selected. It explicitly does **not** patch Claude Code or use
undocumented flags — "Sessions remain fully interactive—users can still
intervene manually at any time." This is a useful adjacent trick if your
auto-typed startup command is going to trigger a permission prompt you also
want to auto-clear.

`claude-squad` (https://github.com/smtg-ai/claude-squad) is a higher-level
tmux session manager for running multiple `claude`/`aider`/`codex` agents
in isolated tmux sessions/git worktrees; its README documents a keybinding
(`N`, capital) for "new session with a prompt," implying built-in initial-
prompt injection into the launched agent, but the underlying `tmux
send-keys` mechanics weren't disclosed in the README content that was
fetched.

**Why `expect`/pty-less automation is a different (and reportedly broken)
story.** A general definition of `expect` for context: it is "a tool for
scripts that control interactive processes" (Don Libes, NIST publication,
https://tsapps.nist.gov/publication/get_pdf.cfm?pub_id=821307) — classic
`expect` scripts spawn the target process under a real pty and then write
into that pty, which is mechanically similar to what `tmux send-keys` does.
However, a **currently open feature request** (issue #15553,
https://github.com/anthropics/claude-code/issues/15553) documents a
specific, sourced failure mode for *some* forms of programmatic input:

> Claude Code uses the Ink library for terminal UI. Ink's `ink-text-input`
> component treats programmatic stdin input differently than physical
> keyboard input: physical Enter keypress triggers `onSubmit` and the
> prompt is processed, while programmatic `\r` or `\n` is treated as a
> newline character and the prompt is NOT submitted.

The reporter (building a voice-to-code integration) tried and documented as
**failing**: VS Code's `terminal.sendText(text, true)`, `sendSequence` with
`\r`/`\x0d`/`\x0a`/unicode variants, and macOS AppleScript
`osascript ... keystroke return` / `key code 36`. All of these either write
through a higher-level terminal API rather than a raw pty, or synthesize an
OS-level keystroke rather than a pty byte stream — and none of them
triggered Claude Code's submit handler. The issue is open with no
maintainer fix yet; proposed remedies (an env var to accept programmatic
`\r`/`\n` as submit, a dedicated escape sequence, or a Unix socket/named
pipe for out-of-band input) are all still just proposals.

**Reconciling the two findings:** the community `tmux send-keys ... C-m`
recipes above are widely reported as *working* for real Claude Code TUI
submission, while VS Code's terminal API and AppleScript keystrokes are
reported as *not* working for the same purpose. `Inferred:` the likely
distinguishing factor is that `tmux send-keys` writes bytes into a genuine
pseudo-terminal device that the child process reads via its normal raw-mode
stdin (so it's indistinguishable from physical typing at the OS/pty layer),
whereas VS Code's `sendText`/`sendSequence` and AppleScript `keystroke` may
route through different injection layers (e.g. VS Code's own terminal
renderer, or macOS Accessibility APIs) that don't land as raw pty bytes in
the same way. No single source states this distinction explicitly for
Claude Code — it's inferred from combining the #15553 report with the
independently-sourced tmux recipes, not a documented fact.

**`screen -X stuff`**: this is `screen`'s equivalent of `tmux send-keys —
literal` (it writes the given string into the target window's input as if
typed). No source discussing Claude Code specifically mentions it — this is
`Inferred` by analogy to `tmux send-keys` given both write into a real pty,
and should be verified empirically (a quick `sleep` + `stuff` + `stuff
"\015"` test) before relying on it, per the same timing gotchas documented
for tmux above (send text and Enter as separate `stuff` calls, with a delay
between them).

### 4b. CLI flags for pre-seeding while staying interactive

Summarizing the mechanisms confirmed above, mapped onto "does it stay
interactive":

| mechanism | stays interactive? | source |
|---|---|---|
| `claude "prompt"` | yes (documented) | cli-reference table |
| `claude -p "prompt"` | no, exits after one response | cli-reference table |
| `claude -c "prompt"` | resumes interactively, but ignores the inline prompt (bug) | issue #3180 |
| `claude -r "session" "prompt"` | yes (documented) | cli-reference table |
| `claude --resume abc123 --fork-session` | yes (documented, new session id) | cli-reference table |
| `cat file \| claude -p "query"` | no, exits after processing | cli-reference / headless docs |
| `cat file \| claude` (no `-p`) pre-filling the input box | not supported — explicitly rejected feature request | issue #6009 |

There is no flag that does "pre-seed the editable input box, but require the
user to press Enter" — the closest documented behavior is that the
positional argument is auto-submitted as the first turn, not staged for
editing.

---

## downloaded files

None — all sources were HTML pages fetched via WebFetch; no binaries were
needed for this research.

---

## sources

### https://code.claude.com/docs/en/cli-reference
Official CLI reference. Contains the flag table with the exact row for the
positional prompt argument (`claude "query"` → "Start interactive session
with initial prompt") and the `--print`/`-p` row ("Print response without
interactive mode"). Also documents `--continue`, `--resume`/`-r`
(`claude -r "session-name" "query"` stays interactive), and
`--resume ... --fork-session`. This is the primary source for angle 1 and 4.

### https://code.claude.com/docs/en/interactive-mode
Full interactive-mode reference: keyboard shortcuts, multiline input,
command history, background bash tasks, `/btw` side questions, session
recap, PR review status. Confirms slash commands are invoked by typing `/`
at the start of input ("Type `/` in Claude Code to see all available
commands"). No explicit statement about the positional-argument startup
path expanding slash commands — used as supporting context for the
`Inferred:` claim in section 1, not as direct evidence.

### https://code.claude.com/docs/en/headless
Official docs for `-p`/print/non-interactive ("Agent SDK via the CLI")
mode. Contains the key verbatim quote confirming slash-command expansion in
prompt strings: "User-invoked skills and custom commands work in `-p`
mode: include `/skill-name` in the prompt string and Claude Code expands it
before running. Built-in commands that only run in the terminal interface,
such as `/login`, aren't available in `-p` mode." Also documents `--bare`
mode, background-task-at-exit behavior, piping stdin (10MB cap as of
v2.1.128), `--output-format`, and `--continue`/`--resume` in print mode.

### https://code.claude.com/docs/en/terminal-config
Official terminal-configuration troubleshooting page. Covers Shift+Enter
handling, Option-as-Meta on macOS, terminal bell/notifications, tmux
passthrough config (`allow-passthrough`, `extended-keys`), theme
customization, fullscreen rendering, and paste handling. Notably: no
mention of programmatic input injection or automation — this page is about
making the *human* typing experience correct, not automating it. Useful to
rule out as a source for angle 3/4.

### https://github.com/anthropics/claude-code/issues/6009
Closed ("not planned") feature request: pipe stdin into `claude` (no `-p`)
to pre-populate the interactive TUI's editable prompt box with piped
content, cursor at the end, so the user can add instructions before
submitting. Explicitly rejected by Anthropic — confirms there is no
supported "stage content in the input box without submitting" mechanism.

### https://github.com/anthropics/claude-code/issues/3180
Closed bug report: `claude -c "prompt"` (continue + inline prompt) resumes
the TUI showing prior history but silently drops the new prompt text,
leaving the session idle. Reported against Claude Code 1.0.44 / macOS 14.3.
No workaround given in the issue itself; `--resume`/`-r` with a query is
documented separately as the working alternative.

### https://github.com/anthropics/claude-code/issues/3844
Bug report (SDK-adjacent but rooted in the shared CLI arg parser): a prompt
string starting with `-` (e.g. `-1 + 2 = ?`) gets misparsed as an unknown
CLI option instead of prompt text. Workaround: insert `--` before the
prompt to force end-of-options per POSIX convention.

### https://github.com/anthropics/claude-code/issues/15553
Open feature request, most load-bearing gotcha for angle 3/4: documents
that Claude Code's Ink-based `ink-text-input` component only treats a
*physical* Enter keypress as submit; programmatic `\r`/`\n` written via VS
Code's `terminal.sendText`/`sendSequence` APIs or macOS AppleScript
`keystroke`/`key code 36` are treated as inserted newlines, not submit
triggers. Documents four failed workaround attempts with code snippets and
four proposed (unimplemented) fixes: an env var to accept programmatic
submit, a special escape sequence, a Unix socket/named pipe, or a
`claude.submitPrompt()` VS Code extension command.

### https://github.com/anthropics/claude-code/issues/12507
Closed bug report: on some HPC/cluster interactive sessions, Claude Code
exits immediately (clean EOF, exit code 0) because subprocesses it spawns
for shell-environment detection inherit and drain the parent's stdin file
descriptor before the interactive prompt loop can read from it — even
though stdin is a genuine tty. Includes strace evidence. `claude -p
"prompt"` sidesteps it (non-interactive path doesn't need to read stdin the
same way). Regression window: 2.0.45 → 2.0.54. Relevant as a "why did my
automated launch exit instead of going interactive" gotcha unrelated to
prompt injection itself.

### https://github.com/anthropics/claude-code-action/issues/523
Bug report against the **GitHub Action** (not the interactive CLI):
passing a slash command as the Action's `prompt:` input causes the run to
halt immediately with zero tokens used and an empty result, instead of
either resolving the command or treating it as literal text. Useful as a
"different code path, don't conflate with the CLI's own `-p`/positional-arg
slash-command handling" caveat.

### https://dev.to/ikramar/i-wrapped-claude-code-in-a-zsh-function-heres-every-decision-i-almost-got-wrong-2ofc
Detailed personal writeup of a ~60-line zsh wrapper function around
`claude`. Best source found for angle 2's "why function, not alias"
reasoning (tty inheritance, subcommand dispatch). Documents real gotchas:
naming collision with `/usr/bin/cc` the C compiler breaking Rust builds,
subcommand-vs-flag ambiguity (`cco "plan my vacation"` misrouting to the
`plan` subcommand), and the `resume` subcommand's TUI picker breaking when
piped through `tee` for logging. Does not inject slash commands
specifically — injects flags (`--append-system-prompt` from a file,
`--permission-mode`) and worktree setup before dispatching to `claude`.

### https://github.com/mushfoo/claude-fish
Fish shell wrapper repo that routes between `claude` and `claude-trace`
based on flags. README describes install-by-copying-files-into-
`~/.config/fish/functions/` pattern and claims full argument passthrough,
but the actual function source wasn't visible in the fetched content — no
evidence it pre-seeds a prompt or slash command; appears to be a pure
routing/passthrough wrapper.

### https://www.hsablonniere.com/dotfiles-claude-code-my-tiny-config-workshop--95d5fr/
Dotfiles blog post covering fish shell config, alias-to-function
conversions, and keyboard shortcuts generally. No `claude`-CLI-specific
wrapper code found despite the promising title — not useful for this
research beyond ruling it out.

### https://gist.github.com/GGPrompts/800f2c67d96bceab836c0090b71488ef
GitHub Gist documenting "pmux," a Claude Code slash command that crafts a
prompt interactively and then delivers it into a *different*, already-
running Claude Code tmux pane via `tmux send-keys`. Primary source for the
canonical tmux injection recipe: `tmux send-keys -t "$PANE" -l "TEXT"`,
then `sleep 0.3`, then `tmux send-keys -t "$PANE" C-m` as a separate call.
Calls the 0.3s delay "CRITICAL" to avoid submitting before the prompt box
has finished loading, especially for long prompts. Also documents pane
discovery via `tmux list-panes -a -F ...` and post-send verification via
capturing the last five lines of the pane.

### https://hboon.com/using-tmux-with-claude-code/
Personal blog post on tmux+Claude Code config. Does **not** describe
automated prompt/command injection via `send-keys` — only passthrough
keybindings (e.g. `bind o send-keys C-o`) for forwarding control-key
combos through tmux to Claude Code, plus `allow-passthrough`/
`extended-keys` tmux settings. Workflow emphasized is manual interactive
use plus `tmux capture-pane` for reading output after the fact, not
programmatic input. Useful as a negative result / to rule out.

### https://crossaitools.com/skills/obra/superpowers-lab/using-tmux-for-interactive-commands
"Superpowers" Claude Code skill/guide specifically about controlling
interactive TUIs via tmux. Second independent source (after the pmux gist)
for the send-text-then-separate-Enter pattern, and adds: prefer polling
`capture-pane` to confirm screen state over a blind fixed sleep, and
"always send Enter as a separate argument" — bundling causes dropped/
misinterpreted keystrokes. General best-practices source for angle 3.

### https://www.devas.life/how-to-run-claude-code-in-a-tmux-popup-window-with-persistent-sessions/
Blog post with a concrete tmux keybinding recipe for launching Claude Code
in a persistent, directory-hashed detached tmux session displayed via
`tmux display-popup`. Confirmed: no initial-prompt/slash-command injection
via `send-keys` is present in this recipe — it launches plain `"claude"`
with no arguments. Useful as a "persistent session" pattern but not a
prompt-injection source.

### https://raw.githubusercontent.com/smtg-ai/claude-squad/main/README.md
README for claude-squad, a terminal app managing multiple AI coding agents
(Claude Code, Codex, OpenCode, Aider) in isolated tmux sessions/git
worktrees. Documents a `N` (capital) keybinding for "new session with a
prompt" vs. lowercase `n` for a plain new session, implying built-in
initial-prompt injection, plus a `-p`/`--program` flag to override the
launched program per-agent. Underlying tmux mechanics not disclosed in the
README content fetched.

### https://raw.githubusercontent.com/claude-yolo/claude-yolo/main/README.md
README for claude-yolo, a tmux-based auto-approval daemon for running
parallel unattended Claude Code agents. Documents its detection+response
loop: `tmux capture-pane` polling (default 0.3s interval) for permission-
dialog markers (Allow/Deny buttons, or `❯`-prefixed numbered options plus
tool-name/"requires approval" keywords), then `tmux send-keys` to confirm
the pre-selected Yes option (or send its number if not pre-selected).
Handles collapsed-transcript detection (sends `Ctrl+O` to expand) and only
auto-answers question dialogs marked `(Recommended)`. Explicitly states it
doesn't patch Claude Code or use undocumented flags, and sessions remain
fully interactive for manual intervention. Useful adjacent technique for
auto-clearing permission prompts triggered by an auto-injected command.
