# Publishing File Flier on Flathub

Once File Flier is on Flathub, people can install it from Bazzite's software
center (or Discover, GNOME Software, Bazaar) and get updates automatically.
Publishing and hosting are free.

This is a one-time setup, about 30 minutes plus Flathub's review time. After
it's done, each new version is a small pull request.

## 1. Prepare the GitHub repo

1. **Rename the repo to `FileFlier`** (Settings → General → Repository name).
   Flathub checks that the app ID `io.github.andrew11155.FileFlier` matches a
   real repo at `github.com/andrew11155/File-Flier`. GitHub redirects the old
   `File-Flier` URLs automatically.
2. **Make the repo public** (Settings → General → Danger Zone → Change
   visibility). Flathub only builds from public source.
3. **Merge the work into your default branch** and create a release tag:
   ```sh
   git tag v0.1.0
   git push origin v0.1.0
   ```
   The screenshot links in `assets/io.github.andrew11155.FileFlier.metainfo.xml`
   point at this tag, so the tag must exist before you submit. Pushing the tag
   also runs the *Release* workflow, which attaches a `.flatpak` bundle and a
   binary tarball to the GitHub release.

## 2. Write the Flathub manifest

Flathub keeps its own copy of the manifest. Copy
`flatpak/io.github.andrew11155.FileFlier.yml` and `flatpak/cargo-sources.json`,
then replace the local `type: dir` source with the tagged release:

```yaml
    sources:
      - type: git
        url: https://github.com/andrew11155/File-Flier.git
        tag: v0.1.0
        commit: <full commit hash of v0.1.0>   # git rev-parse v0.1.0^{commit}
      - cargo-sources.json
```

## 3. Submit

Follow <https://docs.flathub.org/docs/for-app-authors/submission>. In short:

1. Fork <https://github.com/flathub/flathub>. Clone it and check out its
   `new-pr` branch.
2. Create a branch. Add the manifest from step 2 and `cargo-sources.json` at the
   top level.
3. Open a pull request **against `new-pr`** (not `master`), titled
   `Add io.github.andrew11155.FileFlier`.
4. Comment `bot, build` on the pull request to trigger a test build.

### Explaining the permissions

Flathub's linter will flag `finish-args-host-filesystem-access`. Flathub grants
exceptions for this to file managers. Put something like this in the pull
request description:

> File Flier is a file manager, so browsing and managing the user's files is its
> core purpose. `--filesystem=host` lets it access the home folder, removable
> drives under `/run/media` and `/mnt`, and the user's real trash in
> `~/.local/share/Trash`. `xdg-run/gvfs` lets it open network shares that GVFS
> has mounted. `--system-talk-name=org.freedesktop.UDisks2` lets it mount and
> safely eject USB drives from the sidebar, as other file managers do (the
> desktop's polkit agent still asks for a password where required).
> `--share=network` is used only by the built-in updater (a GitHub release check
> and download, which can be switched off in Settings) and for cloud features;
> there is no telemetry. The app does not request
> `org.freedesktop.Flatpak` (host command execution). The optional "Open terminal
> here" feature tells users how to enable that with `flatpak override` if they
> want it.

Flathub does not allow apps to update themselves, so the Flathub build should
disable the updater at compile time by adding `FILE_FLIER_NO_UPDATER: '1'` to
`build-options.env` in the manifest. That hides the Updates settings and the
"Update available" button and makes no requests. With the updater off, the
manifest no longer needs `--share=network` unless you rely on cloud features
that call out, so drop it and the "no network access" claim in the description
becomes true again.

## 4. After it's accepted

- Flathub creates a repo called `flathub/io.github.andrew11155.FileFlier` and
  gives you write access. Log into <https://flathub.org> with GitHub to verify
  that the app is yours.
- **To publish an update:**
  1. Tag a new version in your repo.
  2. Add a `<release>` entry to the metainfo file.
  3. Regenerate `cargo-sources.json` if `Cargo.lock` changed.
  4. In the Flathub repo, update `tag`/`commit` in the manifest and open a pull
     request. When it merges, users get the update automatically.

## Checking locally before submitting

```sh
flatpak install --user flathub org.flatpak.Builder
flatpak run --command=flatpak-builder-lint org.flatpak.Builder manifest flatpak/io.github.andrew11155.FileFlier.yml
flatpak run org.flatpak.Builder --user --force-clean --repo=repo build-dir flatpak/io.github.andrew11155.FileFlier.yml
flatpak run --command=flatpak-builder-lint org.flatpak.Builder repo repo
```

Expect `finish-args-host-filesystem-access` until Flathub grants the exception.
Until the repo is public and tagged, you'll also see `appid-url-not-reachable`
and the screenshot errors. Anything else should be fixed before you submit.
