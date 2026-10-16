ElevenClock-rs - unsigned development candidate

This build is for development validation. Distribution is blocked until the
licensing and Windows acceptance gates in the matching source are resolved.
It is not a supported release. Windows may warn about this unsigned download.

Installer: installs for the current user, without elevation, to
%LOCALAPPDATA%\Programs\ElevenClock-rs. Open ElevenClock-rs from Start.
ZIP: extract to a writable folder and double-click elevenclock-rs.exe.
Do not run legacy ElevenClock at the same time.

Settings opens on interactive launch. Closing Settings leaves the clock running.
Use Exit ElevenClock-rs in Settings or the tray menu to quit and reveal the
Windows clock. Startup is off by default; enable it in Settings > Startup.
Use Settings > Support > Copy diagnostics when reporting a problem.

Exit the app before updating, uninstalling or restoring an older installer.
Uninstall through Windows Settings > Apps > Installed apps. Private preferences
and logs in %LOCALAPPDATA%\ElevenClock-rs are preserved. The legacy ElevenClock
installation and configuration are never removed. A portable copy's startup
registration is preserved when it points outside this installation.

No Python or Qt runtime is included. The MSVC CRT is statically linked; Windows
system DLL imports are listed in runtime-dependencies.json. A clean Windows 11
standard-account run without developer runtimes is still required before release.
No elevated prerequisite installer is run.

LICENSE.txt retains GPLv3. The inherited icon is credited to ElevenClock / Marti
Climent and Xavier Delugo (legacy/README.md); asset and naming review remains
open in https://github.com/tjorim/ElevenClock-rs/issues/32. No relicensing is implied.
THIRD-PARTY-NOTICES.txt and RUST-COPYRIGHT.html retain dependency/toolchain notices.
Matching source, vendored Cargo dependencies, provenance and SHA256 checksums
accompany this candidate as separate artifacts. Keep them together.
