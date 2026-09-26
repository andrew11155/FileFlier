#!/bin/sh
# Install File Flier without root. Works on immutable distros such as Bazzite,
# Fedora Silverblue/Kinoite, SteamOS and other Fedora Atomic / ostree systems,
# because everything goes into your user account, not /usr.
#
#   ./install.sh              Build and install the Flatpak (recommended); from a
#                             release tarball, installs the prebuilt binary instead
#   ./install.sh --native     Install a plain binary into ~/.local
#   ./install.sh --uninstall  Remove both
set -eu

APP_ID=io.github.andrew11155.FileFlier
cd "$(dirname "$0")"

die() {
    echo "error: $*" >&2
    exit 1
}

install_flatpak() {
    command -v flatpak >/dev/null 2>&1 || die "flatpak is not installed"
    [ -f "flatpak/$APP_ID.yml" ] || die "run this from a source checkout (flatpak/$APP_ID.yml is missing)"
    flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
    if command -v flatpak-builder >/dev/null 2>&1; then
        set -- flatpak-builder
    else
        # No flatpak-builder on the host (the norm on immutable systems): use Flathub's.
        echo "Installing org.flatpak.Builder from Flathub..."
        flatpak install --user -y --noninteractive flathub org.flatpak.Builder
        set -- flatpak run org.flatpak.Builder
    fi
    "$@" --user --install --force-clean --disable-rofiles-fuse --install-deps-from=flathub build-dir "flatpak/$APP_ID.yml"
    # Self-installs opt in to "Open terminal here", which runs your terminal outside the
    # sandbox. Undo with: flatpak override --user --reset $APP_ID
    flatpak override --user --talk-name=org.freedesktop.Flatpak "$APP_ID"
    echo
    echo "Installed. Launch \"File Flier\" from your app menu, or run: flatpak run $APP_ID"
}

install_native() {
    bin_dir="$HOME/.local/bin"
    app_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
    if [ -x ./file-flier ]; then
        bin=./file-flier # prebuilt release tarball
    else
        command -v cargo >/dev/null 2>&1 || die "Rust is needed to build from source.
  On Bazzite/Silverblue, build inside a distrobox (it shares your home folder):
    distrobox create -n build -i registry.fedoraproject.org/fedora:latest
    distrobox enter build -- sh -c 'sudo dnf install -y cargo gcc && ./install.sh --native'"
        cargo build --release --locked
        bin=target/release/file-flier
    fi
    install -Dm755 "$bin" "$bin_dir/file-flier"
    # Absolute Exec path: ~/.local/bin isn't always on the launcher's PATH.
    mkdir -p "$app_dir/applications"
    sed "s|^Exec=file-flier|Exec=$bin_dir/file-flier|" "assets/$APP_ID.desktop" >"$app_dir/applications/$APP_ID.desktop"
    install -Dm644 "assets/$APP_ID.svg" "$app_dir/icons/hicolor/scalable/apps/$APP_ID.svg"
    update-desktop-database "$app_dir/applications" >/dev/null 2>&1 || true
    gtk-update-icon-cache -q "$app_dir/icons/hicolor" >/dev/null 2>&1 || true
    echo "Installed $bin_dir/file-flier and a menu entry for \"File Flier\"."
}

uninstall() {
    app_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
    if command -v flatpak >/dev/null 2>&1 && flatpak info --user "$APP_ID" >/dev/null 2>&1; then
        flatpak uninstall --user -y "$APP_ID"
        flatpak override --user --reset "$APP_ID" 2>/dev/null || true
    fi
    rm -f "$HOME/.local/bin/file-flier" \
        "$app_dir/applications/$APP_ID.desktop" \
        "$app_dir/icons/hicolor/scalable/apps/$APP_ID.svg"
    echo "File Flier removed. Settings remain in ~/.config/file-flier (native) or ~/.var/app/$APP_ID (Flatpak)."
}

if [ -f "flatpak/$APP_ID.yml" ]; then default=--flatpak; else default=--native; fi
case "${1:-$default}" in
--flatpak) install_flatpak ;;
--native) install_native ;;
--uninstall) uninstall ;;
-h | --help) sed -n '2,9p' "$0" ;;
*) die "unknown option '$1' (try --help)" ;;
esac
