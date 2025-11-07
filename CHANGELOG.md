# Changelog

## 1.3.11
- Added filter to toggle between enabled services and all services.
- Added the new workspace member services-merge so the installer can use a Rust helper without additional dependencies.
- Implemented helper to load the services.json template and existing cache, overlay template entries, and persist the merged map with directory creation handled automatically.
- Wired the installer to detect the template, resolve the target user’s home, and invoke the helper so ~/.config/runkit/services.json is always updated with template-preferred values.
- Perform package queries to get service descriptions only if the services.json file has no descriptions, then add them if missing.
- Reworked the header to use dedicated start/end boxes with both window-control variants, letting us move other widgets around the active button placement.
- Added a layout watcher that inspects gtk-decoration-layout, reparenting the logo to the opposite side of the buttons on startup and whenever the setting changes at runtime.
- Kept the hamburger menu anchored on the right while giving the logo symmetrical margins so spacing stays consistent whichever side it lands on.
- Set "Refresh services automatically" to off by default.
- Replaced the pkexec CLI with a long-lived D-Bus service that reuses the existing helper logic, gates privileged calls through polkit (switching between tech.geektoshi.Runkit.require_password and keeps JSON responses for the UI.
- Added a blocking D-Bus proxy instead of spawning runkitd to store a new require_password preference (toggled in Preferences).
- Minor bug fixes.
- Added CHANGELOG.md.
- Updated version number.
    
## 1.0.0
- Initial release
