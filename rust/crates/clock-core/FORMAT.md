# R04 formatter handoff

`clock_core::CompiledFormat::parse(&str)` validates and owns chrono tokens.
`render(DateTime<FixedOffset>)` returns `Result<ClockText, FormatError>`;
`cadence()` returns `Second` when `%S` occurs, otherwise `Minute` (including
literal `%%S` and date-only formats). No mutable time, OS queries, shell, HTML
interpretation, arithmetic or offset-expression compatibility is involved.

Settings can use `format::PRESETS`, `CompiledFormat::preview(&[snapshot])`, or
`format::preview(input, &[snapshot])`. Callers supply today's snapshot and a
New Year example; no implicit current time or zone rules are introduced.
Preview results retain each sample's render error. R05 should retain the last
valid preference when parsing/rendering fails; R07/R10/R11 can highlight the
structured `FormatError.kind` and `span`.

| Tokens | Meaning |
| --- | --- |
| `%Y`, `%y` | Calendar year, full / two digits |
| `%G`, `%g` | ISO week-year, full / two digits |
| `%m`, `%d` | Month / day, zero padded |
| `%H`, `%I` | Hour, 24 / 12 hour, zero padded |
| `%M`, `%S`, `%p` | Minute / second, zero padded; English AM/PM |
| `%V`, `%u` | ISO week 01–53; weekday Monday=1 to Sunday=7 |
| `%a`, `%A` | English abbreviated / full weekday |
| `%b`, `%B` | English abbreviated / full month |
| `%%` | Literal percent |

Literal Unicode is supported. One LF or CRLF separates two nonempty lines;
CRLF is normalized to LF. Other controls, empty input/lines, modifiers and every
unlisted directive are rejected. Spaces are literal content, not trimmed.
Input is at most 256 Unicode scalars **before** normalization. Output is at
most 512 scalars **including** the normalized separator. A single checked
formatting pass produces both lines from the supplied snapshot. Overflow
returns an error, never truncated text. An offset that moves a chrono endpoint
outside the representable local-date range returns `SnapshotOutOfRange`.

Error spans are half-open **original-input Unicode scalar indices**, not byte
indices or normalized-string indices. CRLF counts as two source scalars.
Unsupported directives cover `%` plus its next scalar (including modifiers);
a dangling `%` covers one scalar. Controls/newlines cover their source scalars.
Empty lines have zero-width spans. Input overflow identifies 256..257. Render
errors cover the whole input. Example: `🕒\r\n%Q` has span 3..5.

| Date | ISO week date `%g%V.%u` | Calendar year + ISO week `%y%V.%u` |
| --- | --- | --- |
| 2026-10-13 | 2642.2 | 2642.2 |
| 2027-01-01 | 2653.5 | 2753.5 |
| 2027-01-04 | 2701.1 | 2701.1 |
| 2025-12-29 | 2601.1 | 2501.1 |

The third preset, **Time + ISO week date**, is `%H:%M\n%g%V.%u` (an actual
newline in its stored value). All month/weekday names are English initially.
Amsterdam tests supply explicit +01:00/+02:00 snapshots on both sides of the
2026 spring/fall transitions. A fixed offset never implies a timezone rule.

## Development dependencies

Proptest (locked 1.12.0) generates arbitrary Unicode inputs and supported-token
formats with timestamp/offset variation; Criterion (0.7.0, default features off)
measures parsing versus reuse/rendering. Both are development-only and kept
out of the app's normal dependency graph. Existing chrono runtime dependencies
and shared models are unchanged. Both tools are maintained Rust ecosystem
packages, use MIT/Apache-2.0 alternatives, and compile on the existing 1.90 MSRV.
Transitive notice/license validation is separately recorded below; no license
exception or policy expansion is introduced by this work.

## Verification (2026-10-14, Linux x86_64)

The following exact commands passed with both `+1.90.0` and `+stable`
(stable resolved to rustc 1.99.0):

```sh
cargo +1.90.0 fmt --manifest-path rust/Cargo.toml --all --check
cargo +1.90.0 clippy --manifest-path rust/Cargo.toml -p clock-core --all-targets --locked -- -D warnings
cargo +1.90.0 test --manifest-path rust/Cargo.toml -p clock-core --locked
cargo +1.90.0 test --manifest-path rust/Cargo.toml -p clock-core --doc --locked
cargo +stable fmt --manifest-path rust/Cargo.toml --all --check
cargo +stable clippy --manifest-path rust/Cargo.toml -p clock-core --all-targets --locked -- -D warnings
cargo +stable test --manifest-path rust/Cargo.toml -p clock-core --locked
cargo +stable test --manifest-path rust/Cargo.toml -p clock-core --doc --locked
```

Results: 1 core unit test, 8 formatter integration tests (including two
proptest tests, 2,048 cases each), and 2 doc tests passed; no Clippy warnings.
Coverage includes all tokens, Monday/Sunday, ISO week 53, leap day, literal
percent, Unicode, modifier/unlisted/control errors, source spans, exact
input/output bounds including newlines, and chrono range endpoints.

```sh
cargo +1.90.0 bench --manifest-path rust/Cargo.toml -p clock-core --bench format --locked
```

Criterion, 20 samples, 0.5s warmup/1s measurement per function, default two-line
preset: parse interval **577.28–770.57 ns** (central 679.75 ns), render interval
**251.39–333.07 ns** (central 293.66 ns). Shared runner timings are descriptive,
not a performance gate. Parse and render are independently measured; render
reuses the compiled object and allocates owned display lines.

```sh
cargo +stable metadata --manifest-path rust/Cargo.toml --locked --format-version 1 > /workspace/scratch/r04-dependency-metadata.json
python3 scripts/rust-license-check.py /workspace/scratch/r04-dependency-metadata.json > /workspace/scratch/r04-dependency-notices.txt
```

Metadata passed; license checker **failed (exit 1)**. Missing bundled notices:
`anes 0.1.6`, `r-efi 6.0.0`. License expressions outside its exact allowlist:
core_detect, fnv, linux-raw-sys, quick-error, rustix, rusty-fork, same-file,
wait-timeout, walkdir, zerocopy and zerocopy-derive. These require dependency
notice/license review before merge; no failures were suppressed and the shared
license policy/script was not changed in this formatter issue.

- [x] Portable formatter acceptance criteria and benchmark.
- [x] Dependency license/notice gate (resolved in the follow-up below).
- [ ] Hosted Windows MSRV/stable checks; no Windows desktop is attached here.
- [ ] Interactive Settings/overlay checks in downstream UI work. R04 adds no UI;
      explicit-offset DST unit tests do not constitute Windows evidence.

Advisory gate passed (exit 0), using cargo-audit 0.22.2:

```sh
# Working directory: rust/
cargo +stable audit --deny warnings
```

Loaded 1,310 RustSec advisories and scanned 107 locked dependencies; no
vulnerabilities or denied warnings reported. `cargo tree -p clock-core --edges
normal --locked` confirms the runtime graph remains chrono → num-traits only.
`git diff --check` also passed. The initial license/notice failure above was resolved in the follow-up below.


## License/notice checker follow-up (2026-10-14)

The checker now recognizes explicit historical license aliases and reviewed OR
expressions using their MIT alternative. It retains anes 0.1.6's original
release-commit license files and copyright declaration via an offline, version-
and digest-checked manifest; r-efi 6.0.0's original bundled AUTHORS is recognized
by the same checked mapping. No arbitrary terms or missing-notice exemptions
were added. See `docs/RUST_DEPENDENCIES.md` and `docs/dependency-notices/README.md`.

```sh
python3 scripts/test-rust-license-check.py
cargo +stable metadata --manifest-path rust/Cargo.toml --locked --format-version 1 > /workspace/scratch/r04-dependency-metadata.json
python3 scripts/rust-license-check.py /workspace/scratch/r04-dependency-metadata.json > /workspace/scratch/r04-dependency-notices.txt
```

All nine checker regression tests passed. Metadata and full locked dependency
notice inventory both passed (exit 0); original attribution is retained in the
output. The earlier failure record is kept as verification history. Windows
checks remain unverified in this Linux environment.
