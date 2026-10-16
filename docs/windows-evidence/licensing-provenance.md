# Licensing provenance and artwork (#32)

Recorded 2026-10-16 against source base
`6c103039f932b46712f44cdd12c7f65ada66a174` plus this working change.
Status: **Apache 2.0 selected for the original implementation; distribution
remains blocked by product-name review and Windows acceptance**.
The owner stated on 2026-10-16: "We never modified or used the python code itself,
we can keep the fork in legacy gpl and change ours", then selected Apache 2.0.
This owner-supplied authorship statement is the basis for the license decision.
The sampled technical review below is supporting evidence, not an independent
legal determination or an exhaustive audit of authorship.

## Method and scope

Compared the current `rust/crates/**/*.rs` sources (63 files, including tests,
examples and build scripts) with `legacy/elevenclock/**/*.py` (15 files).
An exact whole-line comparison decoded UTF-8, collapsed whitespace using
`' '.join(line.split())`, discarded lines shorter than 32 characters, then
intersected the unique line sets: **7,037 Rust lines, 3,087 Python lines,
zero shared lines**. Reproduce with Python from the repository root:

```python
from pathlib import Path
def lines(paths):
    return {text for path in paths
            for line in path.read_text(encoding="utf-8-sig").splitlines()
            if len(text := " ".join(line.split())) >= 32}
rust = lines(Path("rust/crates").rglob("*.rs"))
legacy = lines(Path("legacy/elevenclock").rglob("*.py"))
print(len(rust), len(legacy), sorted(rust & legacy))
```

This includes comments as literal lines but cannot find translated syntax,
changed comment markers, short text, rearranged tables or adapted algorithms.
The following targeted source inspection therefore compared structure, comments,
constants, preset lists and UI wording as well. It is a sampled technical review,
not an exhaustive derivation analysis of every function or historical revision.

| Area and inspected sources | Observation |
| --- | --- |
| Structure: `legacy/elevenclock/__init__.py` Clock and polling logic; `rust/crates/clock-core/src/state.rs`, `schedule.rs`; `clock-windows/src/runtime.rs` and `runtime/native.rs` | Both replace the taskbar clock and react to visibility/shell changes. Python concentrates behavior in Qt objects and polling; Rust separates an event/effect reducer, scheduling and Win32 adapters. Different organization does not prove independent design: the rewrite plan explicitly cites the Python implementation. |
| Formatting/presets: Python `Clock.loadTimeFormat`, `tools.py::evaluate_expression_string`, `welcome.py::DateTimeFormat`; Rust `clock-core/src/format.rs`, `preferences.rs` | Legacy composes regional formats, optional seconds/weekdays/week numbers, HTML and expression offsets. Rust has a bounded directive allowlist and three explicit ISO-week presets (`%g%V.%u`, `%y%V.%u`, `%H:%M\n%g%V.%u`), with no expression evaluator. No matching three-item preset table was identified in those legacy sections. Shared strftime token names are present. |
| Constants/tables: Python taskbar discovery and settings styles; Rust `clock-windows/src/taskbar/candidate.rs`, `interaction.rs`, `tray.rs`, `theme.rs` | Shared shell class names (`Shell_TrayWnd`, `Shell_SecondaryTrayWnd`) and Windows identifiers remain. Rust defines its own command IDs, 400 ms activation cooldown, queue cap, and opaque light/dark palettes. No copied table was identified in these inspected sections; shared platform identifiers are not evidence of independent provenance. |
| Comments/contracts: Rust module comments in `state.rs`, `format.rs`, `settings_model.rs`, `interaction.rs`, `tray.rs`; related Python clock/settings/menu sections | Rust documents generation ownership, bounded formats, draft commit behavior and native input batching. No verbatim explanatory paragraph was identified in these inspected sections. The whole-line scan above is an additional weak check only. |
| UI wording: `legacy/elevenclock/settings.py`, `tools.py` menu; Rust `clock-windows/src/settings_model.rs`, `interaction.rs` and `clock-app/resources/settings.rc` | Shared short vocabulary includes Settings, Exit, date/time and font terms. Rust uses a four-tab native dialog, explicit Apply/Cancel/Reset flow and ISO-week token help; legacy has a much larger translated Qt settings surface and custom-format expression/HTML help. These sampled passages do not establish an independently authored UI specification. |

The plan's upstream file/line references in `docs/rewrite/plan.md` and
`dissection.md`, fork history, shared product behavior and retained Python source
remain part of the project history. Those facts alone do not establish that
Python implementation code was incorporated into the Rust implementation.
The earlier blanket GPL posture based on that history is superseded by the
owner's authorship clarification and scoped Apache 2.0 decision below.

## Asset changes and checks

- Replaced `assets/icon.ico` with original geometric artwork authored directly in
  `assets/icon.svg`, without reading/tracing an upstream image as an input.
  `scripts/build-icon.cjs` renders the SVG using development-only sharp; it writes
  the Windows ICO and a PNG preview. See `assets/README.md` for terms and sizes.
- The old application icon was byte-identical to `legacy/icon.ico`: SHA256
  `ada188d164f3612cf9224b38cd1fc0583c5001dcd26594bfd843d7d0b21d08bc`.
  That inherited file remains under `legacy/` with the original notices.
- New SVG SHA256:
  `86a79d1831a564de8e7ef878671c7824b78c99d727b0dbfa1077b198c27eac0f`.
  New ICO SHA256:
  `0ac0b9823fd002992819a8f4aaa2e0ab2ec0a4ca59c05dfc0bb757b0219e2f13`.
- Moved all 38 root `media/` files under `legacy/media/`; retained historical
  credits and added a location/provenance note. Root README now uses only the
  original replacement icon. The source companion still intentionally includes
  legacy material and its notices; installed binary/ZIP payloads exclude it.
- Resource 102 and its existing `assets/icon.ico` path remain the application
  and Settings icon. The runtime passes that shared resource to the tray.
  This change does not overwrite previously built candidates in `dist/`;
  they still contain the inherited icon and must not be relabelled as rebuilt.

## Licensing consistency and outstanding decisions

The owner selected **Apache License 2.0** for the independently authored Rust
implementation and original replacement artwork on 2026-10-16. The original
supporting tooling for the rewrite is covered by the same license. This is not
a relicensing of the inherited Python application or upstream artwork.

- Root `LICENSE` contains Apache 2.0. The standard license text was recovered
  from the repository's initial `f030521:LICENSE`; using that text does not
  change the license of later upstream code.
- The previous root GPLv3 text is preserved byte-for-byte at `legacy/LICENSE`.
  Legacy source, imagery and original attribution remain under their existing
  terms. The legacy README now explicitly identifies its historical Apache
  sentence as stale.
- All four Rust crates inherit `license = "Apache-2.0"` and the root license
  file from the workspace. Dependency licenses and notices are unchanged.
- The installer and ZIP copy the root license into `LICENSE.txt`; neither
  includes the legacy application or inherited imagery in its binary payload.
  The matching source companion retains `legacy/LICENSE` and legacy notices.

Root README, dependency policy and installer/ZIP information distinguish
original active artwork from retained upstream material and preserve attribution.
R14's local specification and plan now use the implemented `ElevenClock-rs`
install/data directories; the startup registry value remains `ElevenClockRust`.
The release guide already links the correction on issue #15.

Still required before distribution:

- Product name/listing review (Store, distinct winget identity and trademark
  search). No listing, package ID or name approval is claimed here.
- Review matching source, dependency/toolchain and installer/ZIP notices for
  the chosen release. Existing third-party notices remain in place.

No message was sent upstream and no upstream permission is claimed. This license
decision relies on the owner's statement of independent authorship, not on
permission to relicense upstream material. The historical checklist on GitHub
has not been edited by this task.

`packaging/release-gates.json` retains `licensing.passed = false` for the remaining
product-name review; no Windows acceptance gate is cleared by these changes.

## Validation in this session

- Generated using Node 24.19.0 and sharp 0.35.5; visually inspected the PNG.
- Decoded all nine ICO frames with Pillow: correct dimensions, RGBA, transparent
  corner pixels. Native locked/offline x64 MSVC release build passed using a
  separate `rust/target/licensing` directory.
- Loaded that executable as a data/image resource (without launching the app).
  Resource group 102 contains all nine frames; each embedded RT_ICON PNG is
  byte-identical to the corresponding replacement ICO frame.
- Compared each moved media file against `git show HEAD:media/<name>`:
  all 38 files match byte-for-byte.
- `cargo fmt --manifest-path rust/Cargo.toml --all --check`, specification JSON
  parsing, `git diff --check`, and all eight existing
  `scripts/test-rust-release.py` tests passed. Confirmed licensing gate is false.
- After the Apache 2.0 decision, all four packages' locked/offline Cargo metadata
  reported `Apache-2.0` and resolved their license file to the root Apache text.
  The full dependency/license inventory passed, along with all nine license-check
  tests and all eight release tests. Verified the retained GPL text against the
  previous committed license, allowing only checkout LF/CRLF conversion; the
  legacy file itself was copied directly from the previous working root license.
  Regenerating the artwork after changing its license comment preserved the
  ICO digest above. Formatting, JSON parsing and diff whitespace checks passed.

No installed app, existing candidate package or running clock was replaced.
This is resource/build evidence; the new icon has not been observed in a live
Settings/tray session or a newly assembled installer in this task.
