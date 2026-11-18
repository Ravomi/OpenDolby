#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREFIX_BIN="${XDG_BIN_HOME:-$HOME/.local/bin}"
DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"

APP_DESKTOP="$DATA_HOME/applications/dolby-gui.desktop"
ICON_SVG="$DATA_HOME/icons/hicolor/scalable/apps/opendolby-app.svg"
ICON_PNG="$DATA_HOME/icons/hicolor/256x256/apps/opendolby-app.png"
SYSTEMD_UNIT="$CONFIG_HOME/systemd/user/opendolby.service"
AUTOSTART_DESKTOP="$CONFIG_HOME/autostart/opendolby-autostart.desktop"

usage() {
    cat <<'EOF'
Usage: ./scripts/install.sh [OPTIONS]

Install OpenDolby user-local binaries, desktop launcher, icons, and optional autostart.

Options:
  --enable-autostart   Create XDG desktop autostart entry (starts DSP in background on login)
  --disable-autostart  Remove XDG desktop autostart entry
  --uninstall          Remove installed binaries, desktop entry, icons, and autostart files
  -h, --help           Show this help message
EOF
}

ENABLE_AUTOSTART=0
DISABLE_AUTOSTART=0
UNINSTALL=0

for arg in "$@"; do
    case "$arg" in
        --enable-autostart) ENABLE_AUTOSTART=1 ;;
        --disable-autostart) DISABLE_AUTOSTART=1 ;;
        --uninstall) UNINSTALL=1 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "Unknown option: $arg" >&2; usage >&2; exit 1 ;;
    esac
done

if [[ "$UNINSTALL" -eq 1 ]]; then
    if [[ -x "$PREFIX_BIN/dolby-gui" ]]; then
        "$PREFIX_BIN/dolby-gui" --quit >/dev/null 2>&1 || true
    fi
    rm -f "$PREFIX_BIN/dolby-gui" "$PREFIX_BIN/dolby-rs" \
          "$APP_DESKTOP" "$ICON_SVG" "$ICON_PNG" \
          "$SYSTEMD_UNIT" "$AUTOSTART_DESKTOP"
    echo "Uninstalled OpenDolby user files."
    exit 0
fi

if [[ ! -x "$ROOT_DIR/target/release/dolby-gui" || ! -x "$ROOT_DIR/target/release/dolby-rs" ]]; then
    echo "Release binaries not found; building with cargo..."
    cargo build --manifest-path "$ROOT_DIR/Cargo.toml" --release --locked --bins
fi

install -Dm755 "$ROOT_DIR/target/release/dolby-gui" "$PREFIX_BIN/dolby-gui"
install -Dm755 "$ROOT_DIR/target/release/dolby-rs" "$PREFIX_BIN/dolby-rs"
install -Dm644 "$ROOT_DIR/assets/opendolby-app.svg" "$ICON_SVG"
install -Dm644 "$ROOT_DIR/assets/opendolby.png" "$ICON_PNG"
install -Dm644 "$ROOT_DIR/assets/opendolby.service" "$SYSTEMD_UNIT"

mkdir -p "$(dirname "$APP_DESKTOP")"
sed "s|^Exec=dolby-gui$|Exec=$PREFIX_BIN/dolby-gui|" \
    "$ROOT_DIR/assets/dolby-gui.desktop" > "$APP_DESKTOP"
chmod 0644 "$APP_DESKTOP"

if [[ "$ENABLE_AUTOSTART" -eq 1 ]]; then
    mkdir -p "$(dirname "$AUTOSTART_DESKTOP")"
    cat > "$AUTOSTART_DESKTOP" <<EOF
[Desktop Entry]
Type=Application
Name=OpenDolby (Background Engine)
Comment=Start OpenDolby speaker DSP in background on login
Exec=$PREFIX_BIN/dolby-gui --start --background
Icon=opendolby-app
Terminal=false
X-GNOME-Autostart-enabled=true
EOF
    chmod 0644 "$AUTOSTART_DESKTOP"
    echo "Enabled XDG login autostart: $AUTOSTART_DESKTOP"
elif [[ "$DISABLE_AUTOSTART" -eq 1 ]]; then
    rm -f "$AUTOSTART_DESKTOP"
    echo "Removed XDG login autostart."
fi

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$DATA_HOME/applications" >/dev/null 2>&1 || true
fi

echo "Installed OpenDolby:"
echo "  Binaries : $PREFIX_BIN/dolby-gui, $PREFIX_BIN/dolby-rs"
echo "  Launcher : $APP_DESKTOP"
echo "  Service  : $SYSTEMD_UNIT"
echo "  XML path : $CONFIG_HOME/opendolby/tuning.xml"
