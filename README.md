# Runkit

Graphical manager for Void Linux runit services. The application targets a friendly, guided user experience that balances power-user workflows with newcomers who just want to start, stop, or understand system services.

## Screenshots

<p align="center">
  <img src="assets/screenshots/1-services.png" alt="Runkit Service Manager" width="45%" />
  <img src="assets/screenshots/2-preferences.png" alt="Runkit Preferences" width="45%" />
</p>

## Workspace Layout

- `runkit-core`: service discovery, status parsing, and shared domain types.
- `runkitd`: privileged helper invoked through `pkexec`; executes `sv` commands and manages the `/var/service` symlinks in a controlled manner.
- `runkit`: libadwaita interface that lists services, provides detail panes, and delegates every privileged operation (including status reads) to `runkitd`.

## Requirements

- A distro utilizing runit
- GTK 4.14+ and libadwaita 1.4+ runtimes
- Rust 1.88.0 or newer (tested with 1.88.0)
- Dbus
- Polkit

## Installation

For Void Linux the repository ships an installer that builds release binaries and places them under `/usr/libexec`. It will also install any dependencies, copy icons, and create a runkit.desktop file.

```bash
chmod +x start.sh
./start.sh                 # installs dependencies, builds, and installs binaries
./start.sh uninstall       # removes the installed binaries
```
After installation, you can launch directly from your application launcher or via the CLI by typing runkit.

## Building

This workspace requires the Rust 1.83+ toolchain. The GTK frontend also depends on system libraries:

```bash
sudo xbps-install -S rustup gtk4-devel libadwaita-devel glib-devel pango-devel pkg-config
rustup default stable
```

Once dependencies are present:

```bash
cargo build                # builds every crate
```

> **Note:** `cargo check -p runkit` (or a full `cargo build`) will fail unless the GTK/libadwaita headers are installed. The helper and core crates can be compiled independently with standard Rust tooling.

## Running The App

During development you can bypass `pkexec` and point the UI at a locally built helper:

```bash
cargo build --bins
RUNKITD_PATH=target/debug/runkitd \
RUNKITD_NO_PKEXEC=1 \
  cargo run -p runkit
```

When running normally, `runkit` will invoke the helper for **all** service discovery and lifecycle work, so the first launch will trigger a polkit password prompt. The helper is launched via:

```bash
pkexec /usr/libexec/runkitd <action> <service>
```

so ensure your helper binary and accompanying polkit policy are installed at those paths for production.

### Environment Overrides

The desktop app looks for the following overrides when spawning `runkitd`:

- `RUNKITD_PATH`: full path to the helper binary (defaults to `/usr/libexec/runkitd`).
- `RUNKITD_NO_PKEXEC`: set to `1`/`true` to bypass `pkexec` (useful in development environments).

The legacy `RUNKIT_HELPER_PATH` / `RUNKIT_HELPER_NO_PKEXEC` variables are still honored for compatibility.
