# Notes for Claude

## Attribution

- Do **not** add `Co-Authored-By: Claude …`, `Claude-Session: …`, or any other
  Claude/AI attribution lines to commit messages.
- Do **not** add "Generated with Claude Code" (or similar) footers to pull
  request descriptions, issues or comments.
- Commits are authored as `andrew11155 <89171576+andrew11155@users.noreply.github.com>`.

## Project

File Flier is a File Pilot-style file manager for Linux, written in Rust with
egui. The default branch is `Cloud-main`.

Before pushing, run the same checks as CI (CI uses the latest stable Rust):

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

When `Cargo.lock` changes, regenerate `flatpak/cargo-sources.json`; CI fails if
it's stale (see README → Packaging).

The Flatpak manifest is `flatpak/io.github.andrew11155.FileFlier.yml`, and
Flathub submission steps are in `docs/FLATHUB.md`.

## Subagents

The goal is to use as little of the usage budget as possible. Hand a task to a
cheaper subagent (the Agent tool's `model` parameter) whenever it can reach
the same quality as doing it yourself. Approximate API prices per million
input/output tokens:

| Model | Price | Use it for |
|-------|-------|-----------|
| Haiku 5.5 (`haiku`) | $0.10 / $0.50 (about 40× cheaper than Opus) | Read-only searches and lookups ("where is X handled", "list the callers of Y"), summarising logs or CI output, mechanical edits with an exact spec (renames, adding a field everywhere, regenerating a list), running checks and reporting the results |
| Sonnet 5.5 (`sonnet`) | $2 / $10 (half of Opus) | Well-scoped implementation in one or two files with a clear spec, writing tests, docs and README sections, focused reviews of a diff |
| Opus 5.5 (main session) | $4 / $20 | Design decisions, cross-cutting changes (UI state plus app logic plus platforms), anything subtle (egui interaction, unsafe/FFI, packaging), and final review of subagent work |

- Limits on concurrent subagents: up to **6 Haiku** and up to **3 Sonnet**.
  Don't spawn Opus subagents. Do Opus-level work in the main session.
- Delegate only when the task can be described completely in the prompt.
  Include file paths, the exact change, and how to verify it. A subagent that
  has to rediscover a lot of context, or whose output needs heavy rework,
  costs more than doing the task directly. Small tasks you can do in a few
  tool calls stay in the main session.
- Ask subagents for concise results (findings, file:line, a short diff
  summary), not file dumps.
- Always check subagent output yourself, and run the CI checks above, before
  committing.
