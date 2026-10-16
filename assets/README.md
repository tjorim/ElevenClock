# Application artwork

`icon.svg` is original geometric artwork created for ElevenClock-rs on 2026-10-16:
an ivory clock face, orange hands and a mint taskbar on a navy rounded tile.
It uses no upstream image, font, traced outline or external illustration.
The SVG and its generated `icon.ico` / `icon.png` are supplied under
[Apache License 2.0](../LICENSE), like the original Rust implementation.
The inherited material under `legacy/` retains its own license. This license
choice does not grant rights to the ElevenClock name.

To regenerate, make the development-only Node package `sharp@0.35.5` available and run
`node scripts/build-icon.cjs`. Node and sharp are not application dependencies.
The ICO contains transparent 32-bit PNG frames at 16, 20, 24, 32, 40, 48, 64, 128
and 256 pixels. The resource compiler embeds it as resource 102, which Settings
and the tray already load. No Rust resource IDs or paths need to change.

The inherited icon remains at `legacy/icon.ico`; upstream banners, screenshots
and artwork remain under `legacy/media/`, with their historical notices.
See the [provenance review](../docs/windows-evidence/licensing-provenance.md).
