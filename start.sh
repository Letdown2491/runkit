#!/usr/bin/env bash
set -euo pipefail

SCRIPT_NAME=$(basename "$0")
ACTION=${1:-install}

DEST_DIR="/usr/libexec"
BINARIES=(
    "runkit"
    "runkitd"
)

ICON_BASE_NAME="runkit"
ICON_SOURCE_DIR="assets/icons/hicolor"
ICON_TARGET_BASE="/usr/share/icons/hicolor"
ICON_SIZES=(16x16 24x24 32x32 48x48 64x64 96x96 128x128 256x256 512x512)

DESKTOP_SOURCE="assets/applications/tech.geektoshi.Runkit.desktop"
DESKTOP_TARGET="/usr/share/applications/tech.geektoshi.Runkit.desktop"
DBUS_SERVICE_SOURCE="assets/dbus-1/services/tech.geektoshi.Runkit.service"
DBUS_SERVICE_TARGET="/usr/share/dbus-1/services/tech.geektoshi.Runkit.service"
DBUS_SYSTEM_SERVICE_SOURCE="assets/dbus-1/system-services/tech.geektoshi.Runkit1.service"
DBUS_SYSTEM_SERVICE_TARGET="/usr/share/dbus-1/system-services/tech.geektoshi.Runkit1.service"
DBUS_SYSTEM_CONFIG_SOURCE="assets/dbus-1/system.d/tech.geektoshi.Runkit1.conf"
DBUS_SYSTEM_CONFIG_TARGET="/etc/dbus-1/system.d/tech.geektoshi.Runkit1.conf"
POLKIT_POLICY_SOURCE="assets/polkit-1/actions/tech.geektoshi.Runkit.policy"
POLKIT_POLICY_TARGET="/usr/share/polkit-1/actions/tech.geektoshi.Runkit.policy"
SERVICE_DESCRIPTIONS_TEMPLATE="assets/config/services.json"

require_sudo() {
    sudo -v
}

install_dependencies() {
    local deps=(
        rustup
        gtk4-devel
        libadwaita-devel
        glib-devel
        pango-devel
        pkg-config
    )

    if ! command -v xbps-query >/dev/null 2>&1; then
        echo "xbps-query not found; installing all dependencies..."
        sudo xbps-install -S "${deps[@]}"
        return
    fi

    local missing=()
    echo "Checking system dependencies..."
    for dep in "${deps[@]}"; do
        if xbps-query -p pkgver "$dep" >/dev/null 2>&1; then
            echo "  already installed: ${dep}"
        else
            echo "  missing: ${dep}"
            missing+=("$dep")
        fi
    done

    if (( ${#missing[@]} > 0 )); then
        echo "Installing missing dependencies: ${missing[*]}"
        sudo xbps-install -S "${missing[@]}"
    else
        echo "All dependencies already satisfied."
    fi
}

build_binaries() {
    echo "Building packages..."
    if [[ -n "${SUDO_USER:-}" && "${SUDO_USER}" != "root" ]]; then
        sudo -u "$SUDO_USER" bash -lc '
            if [[ -f "$HOME/.cargo/env" ]]; then
                source "$HOME/.cargo/env"
            fi
            if ! command -v cargo >/dev/null 2>&1; then
                echo "Error: cargo not found in PATH for ${USER}. Ensure Rust is installed (rustup) and try again." >&2
                exit 1
            fi
            cargo build --release
        '
    else
        if ! command -v cargo >/dev/null 2>&1; then
            if [[ -f "$HOME/.cargo/env" ]]; then
                # shellcheck disable=SC1090
                source "$HOME/.cargo/env"
            fi
        fi
        if ! command -v cargo >/dev/null 2>&1; then
            echo "Error: cargo not found in PATH. Ensure Rust is installed (rustup) and try again." >&2
            exit 1
        fi
        cargo build --release
    fi
}

install_binaries() {
    local src_dir="target/release"

    if [[ ! -d "$src_dir" ]]; then
        echo "Error: Source directory '$src_dir' does not exist."
        exit 1
    fi

    for bin in "${BINARIES[@]}"; do
        local src_path="${src_dir}/${bin}"
        local dest_path="${DEST_DIR}/${bin}"

        if [[ ! -f "$src_path" ]]; then
            echo "Warning: '$src_path' not found – skipping."
            continue
        fi

        echo "Installing '$src_path' → '$dest_path'..."
        sudo install -m755 "$src_path" "$DEST_DIR"
    done
}

install_service_descriptions() {
    local template="$SERVICE_DESCRIPTIONS_TEMPLATE"
    if [[ ! -f "$template" ]]; then
        echo "Note: service description template not found at ${template}; skipping."
        return
    fi

    local merger_binary="target/release/services-merge"
    if [[ ! -x "$merger_binary" ]]; then
        echo "Warning: services-merge helper not found at ${merger_binary}; skipping service description merge."
        return
    fi

    local target_user="${SUDO_USER:-$USER}"
    if [[ -z "$target_user" ]]; then
        echo "Warning: unable to determine target user for service descriptions; skipping."
        return
    fi

    local target_home
    if ! target_home=$(getent passwd "$target_user" | cut -d: -f6); then
        echo "Warning: unable to resolve home directory for user '${target_user}'; skipping service description merge."
        return
    fi

    if [[ -z "$target_home" ]]; then
        echo "Warning: resolved empty home directory for user '${target_user}'; skipping service description merge."
        return
    fi

    local target_file="${target_home}/.config/runkit/services.json"
    echo "Merging service descriptions into '${target_file}' for user '${target_user}'..."

    if [[ "$target_user" == "$USER" ]]; then
        if ! "$merger_binary" --template "$template" --target "$target_file"; then
            echo "Warning: failed to merge service descriptions for ${target_user}."
        fi
    else
        if ! sudo -u "$target_user" "$merger_binary" --template "$template" --target "$target_file"; then
            echo "Warning: failed to merge service descriptions for ${target_user}."
        fi
    fi
}

install_icons() {
    local installed_any=false
    local missing_sizes=()

    for size in "${ICON_SIZES[@]}"; do
        local src="${ICON_SOURCE_DIR}/${size}/apps/${ICON_BASE_NAME}.png"
        if [[ -f "$src" ]]; then
            local dest="${ICON_TARGET_BASE}/${size}/apps/${ICON_BASE_NAME}.png"
            echo "Installing icon '$src' -> '$dest'..."
            sudo install -D -m644 "$src" "$dest"
            installed_any=true
        else
            missing_sizes+=("$size")
        fi
    done

    if (( ${#missing_sizes[@]} > 0 )); then
        echo "Note: PNG icons missing for sizes: ${missing_sizes[*]}"
        echo "      Place files under ${ICON_SOURCE_DIR}/<size>/apps/${ICON_BASE_NAME}.png"
    fi

    local svg_src="${ICON_SOURCE_DIR}/scalable/apps/${ICON_BASE_NAME}.svg"
    if [[ -f "$svg_src" ]]; then
        local svg_dest="${ICON_TARGET_BASE}/scalable/apps/${ICON_BASE_NAME}.svg"
        echo "Installing icon '$svg_src' -> '$svg_dest'..."
        sudo install -D -m644 "$svg_src" "$svg_dest"
        installed_any=true
    else
        echo "Note: scalable icon missing at ${svg_src}"
    fi

    if [[ "$installed_any" == true ]]; then
        refresh_icon_cache
    fi
}

install_desktop_entry() {
    if [[ -f "$DESKTOP_SOURCE" ]]; then
        echo "Installing desktop entry '$DESKTOP_SOURCE' -> '$DESKTOP_TARGET'..."
        sudo install -D -m644 "$DESKTOP_SOURCE" "$DESKTOP_TARGET"
        refresh_desktop_database
    else
        echo "Note: desktop entry not found at ${DESKTOP_SOURCE}; skipping."
    fi
}

install_dbus_service() {
    if [[ -f "$DBUS_SERVICE_SOURCE" ]]; then
        echo "Installing D-Bus service '$DBUS_SERVICE_SOURCE' -> '$DBUS_SERVICE_TARGET'..."
        sudo install -D -m644 "$DBUS_SERVICE_SOURCE" "$DBUS_SERVICE_TARGET"
    else
        echo "Note: D-Bus service file not found at ${DBUS_SERVICE_SOURCE}; skipping."
    fi

    if [[ -f "$DBUS_SYSTEM_SERVICE_SOURCE" ]]; then
        echo "Installing system D-Bus service '$DBUS_SYSTEM_SERVICE_SOURCE' -> '$DBUS_SYSTEM_SERVICE_TARGET'..."
        sudo install -D -m644 "$DBUS_SYSTEM_SERVICE_SOURCE" "$DBUS_SYSTEM_SERVICE_TARGET"
    else
        echo "Note: system D-Bus service file not found at ${DBUS_SYSTEM_SERVICE_SOURCE}; skipping."
    fi

    if [[ -f "$DBUS_SYSTEM_CONFIG_SOURCE" ]]; then
        echo "Installing system D-Bus policy '$DBUS_SYSTEM_CONFIG_SOURCE' -> '$DBUS_SYSTEM_CONFIG_TARGET'..."
        sudo install -D -m644 "$DBUS_SYSTEM_CONFIG_SOURCE" "$DBUS_SYSTEM_CONFIG_TARGET"
    else
        echo "Note: system D-Bus policy not found at ${DBUS_SYSTEM_CONFIG_SOURCE}; skipping."
    fi
}

install_polkit_policy() {
    if [[ -f "$POLKIT_POLICY_SOURCE" ]]; then
        echo "Installing polkit policy '$POLKIT_POLICY_SOURCE' -> '$POLKIT_POLICY_TARGET'..."
        sudo install -D -m644 "$POLKIT_POLICY_SOURCE" "$POLKIT_POLICY_TARGET"
    else
        echo "Note: polkit policy not found at ${POLKIT_POLICY_SOURCE}; skipping."
    fi
}

uninstall_icons() {
    local removed_any=false

    for size in "${ICON_SIZES[@]}"; do
        local dest="${ICON_TARGET_BASE}/${size}/apps/${ICON_BASE_NAME}.png"
        if [[ -f "$dest" ]]; then
            echo "Removing icon '$dest'..."
            sudo rm -f "$dest"
            removed_any=true
        fi
    done

    local svg_dest="${ICON_TARGET_BASE}/scalable/apps/${ICON_BASE_NAME}.svg"
    if [[ -f "$svg_dest" ]]; then
        echo "Removing icon '$svg_dest'..."
        sudo rm -f "$svg_dest"
        removed_any=true
    fi

    if [[ "$removed_any" == true ]]; then
        refresh_icon_cache
    fi
}

refresh_icon_cache() {
    if command -v gtk-update-icon-cache >/dev/null 2>&1; then
        echo "Updating icon cache..."
        sudo gtk-update-icon-cache -f "$ICON_TARGET_BASE"
    fi
}

uninstall_desktop_entry() {
    if [[ -f "$DESKTOP_TARGET" ]]; then
        echo "Removing desktop entry '$DESKTOP_TARGET'..."
        sudo rm -f "$DESKTOP_TARGET"
        refresh_desktop_database
    fi
}

uninstall_dbus_service() {
    if [[ -f "$DBUS_SERVICE_TARGET" ]]; then
        echo "Removing D-Bus service '$DBUS_SERVICE_TARGET'..."
        sudo rm -f "$DBUS_SERVICE_TARGET"
    fi
    if [[ -f "$DBUS_SYSTEM_SERVICE_TARGET" ]]; then
        echo "Removing system D-Bus service '$DBUS_SYSTEM_SERVICE_TARGET'..."
        sudo rm -f "$DBUS_SYSTEM_SERVICE_TARGET"
    fi
    if [[ -f "$DBUS_SYSTEM_CONFIG_TARGET" ]]; then
        echo "Removing system D-Bus policy '$DBUS_SYSTEM_CONFIG_TARGET'..."
        sudo rm -f "$DBUS_SYSTEM_CONFIG_TARGET"
    fi
}

uninstall_polkit_policy() {
    if [[ -f "$POLKIT_POLICY_TARGET" ]]; then
        echo "Removing polkit policy '$POLKIT_POLICY_TARGET'..."
        sudo rm -f "$POLKIT_POLICY_TARGET"
    fi
}

refresh_desktop_database() {
    if command -v update-desktop-database >/dev/null 2>&1; then
        local dir
        dir=$(dirname "$DESKTOP_TARGET")
        echo "Refreshing desktop database for $dir..."
        sudo update-desktop-database "$dir"
    fi
}

uninstall_binaries() {
    echo "Removing installed binaries..."

    for bin in "${BINARIES[@]}"; do
        local dest_path="${DEST_DIR}/${bin}"

        if [[ ! -f "$dest_path" ]]; then
            echo "Skipping '${dest_path}'; not present."
            continue
        fi

        echo "Removing '$dest_path'..."
        sudo rm -f "$dest_path"
    done
}

case "$ACTION" in
    install)
        require_sudo
        install_dependencies
        build_binaries
        install_binaries
    install_icons
    install_desktop_entry
    install_dbus_service
    install_polkit_policy
    install_service_descriptions
        ;;
    uninstall)
        require_sudo
        uninstall_binaries
        uninstall_icons
        uninstall_desktop_entry
        uninstall_dbus_service
        uninstall_polkit_policy
        ;;
    *)
        echo "Usage: $SCRIPT_NAME [install|uninstall]" >&2
        exit 1
        ;;
esac

echo "Done."
