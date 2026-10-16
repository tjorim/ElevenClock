# Documentation

Docs for the Rust rewrite of ElevenClock (`ElevenClock-rs`). The legacy Python app is under
[`legacy/`](../legacy) and is not covered here.

| Folder / file | What it is | Changes when |
| --- | --- | --- |
| [`contributing.md`](contributing.md) | Workspace layout, commands, CI and the contracts contributors must keep | The workflow changes |
| [`dependencies.md`](dependencies.md), [`dependency-notices/`](dependency-notices) | Dependency policy and bundled third-party notices | A dependency is added, removed or updated |
| [`releasing.md`](releasing.md) | Native x64 packaging, installer ownership and release gates | Packaging changes |
| [`design/`](design) | How each part behaves and why: the interfaces, states and limits a reader needs | The behaviour changes |
| [`adr/`](adr) | Architecture decisions that were weighed and settled | A decision is made or superseded |
| [`windows-evidence/`](windows-evidence) | Dated records from real Windows runs: environments, screenshots, measurements, and the acceptance results (`acceptance.md`) | A new run happens. Never edited to say something that was not observed |
| [`rewrite/`](rewrite) | Rewrite planning scaffolding: plan, source dissection, issue index and issue specs | Removed together at cutover (R15) |

## `design/`

| Topic | File |
| --- | --- |
| Clock state, scheduling and effects (pure core) | [`state-and-scheduling.md`](design/state-and-scheduling.md) |
| Saved preferences: encoding, validation, recovery | [`preferences.md`](design/preferences.md) |
| DirectWrite rendering, DPI, theme, overflow | [`rendering.md`](design/rendering.md) |
| Runtime, monitors, Explorer recovery, shutdown | [`lifecycle.md`](design/lifecycle.md) |
| Clock actions, tray, keyboard and accessibility | [`interaction.md`](design/interaction.md) |
| Settings window and the preference flow | [`settings.md`](design/settings.md) |
| Support status, Copy diagnostics, local logs | [`diagnostics.md`](design/diagnostics.md) |
| Opt-in login startup and the uninstall contract | [`startup.md`](design/startup.md) |

Each design file still ends with the verification that was done when it was written and
the checks that were left open. Treat those sections as history; the current state of a
release gate is in `windows-evidence/`.

## `windows-evidence/`

| Topic | File | Raw artifacts |
| --- | --- | --- |
| Taskbar discovery | [`discovery.md`](windows-evidence/discovery.md) | [`discovery-hosted/`](windows-evidence/discovery-hosted) |
| Overlay feasibility and recovery | [`overlay-spike.md`](windows-evidence/overlay-spike.md) | [`overlay-hosted/`](windows-evidence/overlay-hosted), [`overlay-recovery-hosted/`](windows-evidence/overlay-recovery-hosted) |
| Native Settings foundation (DPI, high contrast, keyboard) | [`settings-foundation.md`](windows-evidence/settings-foundation.md) | [`settings-foundation-matrix/`](windows-evidence/settings-foundation-matrix) |
| Packaging and installer lifecycle | [`packaging.md`](windows-evidence/packaging.md) | `native-evidence/packaging/` (generated CI artifact) |

Screenshots used by a design file live next to it, in [`design/images/`](design/images).

## Naming

Files are named by topic, not by issue number. Issue IDs (R01 to R15) appear inside the
text as cross-references only, because the numbering belongs to the rewrite plan, which is
removed at cutover.
