# Runkit

Graphical manager runit services. The application targets a friendly, guided user experience that balances power-user workflows with newcomers who just want to start, stop, or understand system services. While it was written for Void Linux, it should work on any disto which uses runit.

## Screenshots

<p align="center">
  <img src="assets/screenshots/1-services.png" alt="Runkit Service Manager" width="45%" />
  <img src="assets/screenshots/2-preferences.png" alt="Runkit Preferences" width="45%" />
</p>

## Workspace Layout

- `runkit-core`: service discovery, status parsing, and shared domain types.
- `runkitd`: privileged helper exposed as the system D-Bus service `tech.geektoshi.Runkit1`. It runs as root, executes `sv` commands, manages `/var/service` symlinks, and enforces polkit authorization per request.
- `runkit`: libadwaita interface that lists services, shows details, and calls into the D-Bus helper for status queries and lifecycle operations.
- `services-merge`: tiny utility used by the installer to seed and merge cached service descriptions.

## Requirements

- A distro utilizing runit
- GTK 4.14+ and libadwaita 1.4+ runtimes
- Rust 1.88.0 or newer (tested with 1.88.0)
- DBus
- Polkit

## Installation

For Void Linux the repository ships an installer that builds release binaries and places them under `/usr/libexec`. It will also install any dependencies, copy icons, lay down the desktop entry, seed service descriptions, install the system D-Bus definition, and copy the polkit policy.

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

## Running / Developing

After installation the system bus activates `runkitd` automatically. The desktop app talks to the service using the well-known name `tech.geektoshi.Runkit1`, so the first privileged action prompts through polkit. Users can choose between “always ask” and “reuse authorization while the app is open” in Preferences, which simply toggles the polkit action (`tech.geektoshi.Runkit.require_password` vs `tech.geektoshi.Runkit.cached`).

For local development:

1. Build the helper and GUI:
   ```bash
   cargo build --bins
   ```
2. Start the D-Bus service as root (in another terminal):
   ```bash
   sudo target/debug/runkitd --dbus-service
   ```
3. Run the GUI against the service:
   ```bash
   cargo run -p runkit
   ```

Alternatively, copy `assets/dbus-1/system-services/tech.geektoshi.Runkit1.service` to `/usr/share/dbus-1/system-services/`, set `Exec` to your debug path, and reload the bus.
