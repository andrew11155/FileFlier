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
