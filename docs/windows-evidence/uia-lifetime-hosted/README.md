# Hosted UIA/COM lifetime evidence

Verified 2026-10-16 for [issue #35](https://github.com/tjorim/ElevenClock-rs/issues/35).
The [owner's correction](https://github.com/tjorim/ElevenClock-rs/issues/35#issuecomment-6206538832)
identifies the remaining required work as confirming the Server 2025 x64 UIA
step after #33, and reconciling ADR 0002. The provider error/geometry extension
is optional. Both required follow-ups are now recorded here and in
[ADR 0002](../../adr/0002-same-executable-discovery-helper.md).

## Source and retained evidence

[Runtime lifecycle run 38839269796](https://github.com/tjorim/ElevenClock-rs/actions/runs/38839269796),
PR #33 head `c0c2541139e9e51040fe38e9c34c5cc2e52963d8`, completed successfully.
The **UIA lifetime evidence (controlled provider and real Explorer restart)**
step ran and passed on every matrix target; it was not skipped on Server.

| Runner / binary | Job | Image / version | Local evidence |
| --- | --- | --- | --- |
| Server 2025 / native x64 | [116775350693](https://github.com/tjorim/ElevenClock-rs/actions/runs/38839269796/job/116775350693) | windows-2025-vs2026 / 20261007.264.1 | [log](server-x64/uia-step.log), [JSON](server-x64/evidence.json) |
| Windows 11 ARM64 / native ARM64 | [116775350563](https://github.com/tjorim/ElevenClock-rs/actions/runs/38839269796/job/116775350563) | windows-11-vs2026-arm64 / 20261013.183.4 | [log](windows11-arm-native/uia-step.log), [JSON](windows11-arm-native/evidence.json) |
| Windows 11 ARM64 / emulated x64 | [116775350770](https://github.com/tjorim/ElevenClock-rs/actions/runs/38839269796/job/116775350770) | windows-11-vs2026-arm64 / 20261013.183.4 | [log](windows11-arm-x64-emulated/uia-step.log), [JSON](windows11-arm-x64-emulated/evidence.json) |

`uia-step.log` is the unedited timestamped UIA step section of each downloaded
GitHub job log. `evidence.json` extracts its five evidence records verbatim;
the harness's duplicate final Explorer correlation record is omitted. No test
was rerun locally to produce these files. The workflow also uploaded the full
`native-evidence/` artifacts (14-day retention):

| Artifact | ID |
| --- | --- |
| r08-lifecycle-windows-2025-x86_64-pc-windows-msvc | 12020896401 |
| r08-lifecycle-windows-11-arm-aarch64-pc-windows-msvc | 12021105970 |
| r08-lifecycle-windows-11-arm-x86_64-pc-windows-msvc | 12020413832 |

## Observed behaviour

| Measurement | Server x64 | ARM64 native | x64 emulated |
| --- | --- | --- | --- |
| Healthy inventory | 159 ms, 11 calls, no failures | 204 ms, 11 calls, no failures | 194 ms, 11 calls, no failures |
| Blocked inventory, 4000 ms deadline | `Deadline`, 4018 ms | `Deadline`, 4018 ms | `Deadline`, 4015 ms |
| Helpers spawned / exits observed for blocked inventory | 1 / 1 | 1 / 1 | 1 / 1 |
| Provider killed during outstanding `ElementFromHandle` | `S_OK` | `S_OK` | `S_OK` |
| Retained provider elements after death | both calls `0x80040201` | both calls `0x80040201` | both calls `0x80040201` |
| Explorer call held at termination | `CurrentClassName`, 508 ms | `FindAll`, 508 ms | `FindAll`, 507 ms |
| Outstanding Explorer call returned after termination | `0x8000FFFF` | `0x8000FFFF` | `0x8000FFFF` |
| Retained Explorer calls | 6 failures, `0x80040201` | 8 failures, `0x80040201` | 8 failures, `0x80040201` |
| Old → recovered shell PID | 3756 → 2100 | 7696 → 1072 | 2360 → 8752 |
| Correlated restart and fresh-client recovery | passed, attempt 1 | passed, attempt 1 | passed, attempt 1 |

All five scenarios completed successfully on all targets. The hung-provider
scenario also asserts recovery with a fresh provider and request. All three
Explorer cases needed a manual shell start by the harness after its 45-second
observation period; this does not prove Windows' automatic restart timing.

## What this establishes and what remains separate

The test-only STATIC window's non-pumping UI thread causes real UIA/COM calls
to block. The production `taskbar::uia::inventory` and `worker::run_helper`
bound that inventory by terminating and reaping the helper. The Explorer test
uses real taskbar UIA objects, deliberately suspends Explorer to establish an
outstanding call, then terminates it. It measures per-call timestamps, HRESULTs,
retained-object failure and fresh-client recovery. It is not evidence of a
naturally occurring Explorer hang or an intrinsic COM cancellation mechanism.

Generation rejection, complete-report rejection at deadline and repeated
failure/resource accounting remain covered separately by `discovery_worker.rs`
and the runtime's scripted-platform tests. The Explorer trace client is not the
production discovery helper and does not directly prove stale-generation
rejection during that same COM call. These retained records do not contain
generation-tagged per-call inventory traces or visual assertions about native
clock/overlay visibility. They must not be described as that stronger evidence.

A custom taskbar-like COM provider returning selected HRESULTs (`E_FAIL`,
`UIA_E_ELEMENTNOTAVAILABLE`, `RPC_E_DISCONNECTED`) or stale/garbage geometry is
not implemented by this harness. That remains the optional extension in the
owner's corrected issue scope. Physical-desktop overlay recovery and the
two-display defects remain in [#34](https://github.com/tjorim/ElevenClock-rs/issues/34).
This record completes #35's required confirmation without expanding those claims.

## Reproduction on a disposable Windows runner

The existing [workflow](../../../.github/workflows/runtime-lifecycle.yml) prepares
the hosted desktop, runs the lifecycle checks, then invokes:

```powershell
$env:ELEVENCLOCK_RS_UIA_EXPLORER = '1'
$env:ELEVENCLOCK_RS_EVIDENCE_DIR = 'native-evidence/uia'
cargo +stable test --manifest-path rust/Cargo.toml -p clock-windows --test uia_lifetime --target x86_64-pc-windows-msvc --locked
```

Use `aarch64-pc-windows-msvc` for the native ARM64 case. The opt-in Explorer
scenario suspends and restarts the shell; use a disposable hosted desktop.
The provider, clients and runner are roles of the integration-test executable,
not shipped binaries or code linked into `clock-app`.
