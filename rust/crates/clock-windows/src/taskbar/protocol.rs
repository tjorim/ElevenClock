//! Wire contract between the app and its private discovery helper (R08).
//!
//! The helper owns every HWND-adjacent blocking call and COM interface; only this
//! value-only JSON crosses the process boundary. The parent trusts none of it:
//! the envelope must match the requested generation, sizes are bounded, rectangles
//! are re-validated through `PhysicalRect::new`, and a clock rectangle is only
//! produced when the support [`Gate`] allows it.
use super::gate::Gate;
use crate::runtime::{DiscoveryFailure, ShellIdentity};
use clock_core::{
    DiscoveryStrategy, MonitorId, PhysicalRect, ShellGeneration, TaskbarId, TaskbarKey,
    TaskbarSnapshot, TaskbarVisibility,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const SCHEMA: u64 = 2;
/// A real report is a few hundred bytes per taskbar. Anything near this bound is
/// a misbehaving helper, not data.
pub const MAX_OUTPUT_BYTES: u64 = 256 * 1024;
pub const MAX_TASKBARS: usize = 16;
pub const MAX_ADJACENT: usize = 64;

/// `[left, top, width, height]` in signed physical pixels.
pub type RectWire = [i64; 4];

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShellRecord {
    pub pid: u32,
    pub started: u64,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClockRecord {
    pub rect: RectWire,
    pub adjacent: Vec<RectWire>,
    pub exclusions_complete: bool,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskbarRecord {
    pub id: u64,
    /// The helper's window handle, kept only by the UI thread's native layer and
    /// revalidated (class, owning process) before every use.
    pub hwnd: u64,
    pub taskbar: RectWire,
    pub monitor_id: u64,
    pub monitor: RectWire,
    pub dpi: u32,
    pub primary: bool,
    pub visible: bool,
    pub clock: Option<ClockRecord>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema: u64,
    pub generation: u64,
    pub shell: Option<ShellRecord>,
    pub taskbars: Vec<TaskbarRecord>,
}

/// A validated report: snapshots for the reducer, plus the native layer's private
/// handle table (never part of a core event).
#[derive(Debug)]
pub struct Parsed {
    pub shell: Option<ShellIdentity>,
    pub snapshots: Vec<TaskbarSnapshot>,
    pub handles: Vec<(TaskbarKey, u64)>,
}

pub fn rect_to_wire(rect: PhysicalRect) -> RectWire {
    [
        i64::from(rect.left()),
        i64::from(rect.top()),
        i64::from(rect.width()),
        i64::from(rect.height()),
    ]
}
fn rect_from_wire(wire: RectWire) -> Result<PhysicalRect, DiscoveryFailure> {
    let edge = |value: i64| {
        i32::try_from(value).map_err(|_| rejected("rectangle edge outside the 32-bit range"))
    };
    PhysicalRect::new(
        edge(wire[0])?,
        edge(wire[1])?,
        edge(wire[2])?,
        edge(wire[3])?,
    )
    .map_err(|_| rejected("invalid rectangle"))
}
fn rejected(why: &str) -> DiscoveryFailure {
    DiscoveryFailure::Rejected(why.into())
}

/// Decide a taskbar's clock rectangle. With the gate closed this is always
/// `None`, whatever the helper claims: the support decision is the parent's, not
/// the helper's (see `taskbar::gate`).
fn clock_rect(
    gate: Gate,
    snapshot: &TaskbarSnapshot,
    generation: ShellGeneration,
    record: Option<&ClockRecord>,
) -> Result<Option<PhysicalRect>, DiscoveryFailure> {
    let Some(record) = record else {
        return Ok(None);
    };
    match gate {
        Gate::Closed => Ok(None),
        Gate::Experimental => {
            if record.adjacent.len() > MAX_ADJACENT {
                return Err(rejected("too many neighbouring controls"));
            }
            let candidate = rect_from_wire(record.rect)?;
            let adjacent = record
                .adjacent
                .iter()
                .map(|wire| rect_from_wire(*wire))
                .collect::<Result<Vec<_>, _>>()?;
            // Geometric validation only; identity remains experimental.
            Ok(super::geometry::validate_candidate(
                snapshot,
                generation,
                candidate,
                &adjacent,
                record.exclusions_complete,
            )
            .ok()
            .map(|()| candidate))
        }
    }
}

pub fn parse_report(
    bytes: &[u8],
    requested: ShellGeneration,
    gate: Gate,
) -> Result<Parsed, DiscoveryFailure> {
    if bytes.len() as u64 > MAX_OUTPUT_BYTES {
        return Err(rejected("oversized output"));
    }
    let report: Report = serde_json::from_slice(bytes).map_err(|_| rejected("malformed output"))?;
    if report.schema != SCHEMA {
        return Err(rejected("unsupported schema"));
    }
    if report.generation != requested.0 {
        return Err(rejected("generation mismatch"));
    }
    if report.taskbars.len() > MAX_TASKBARS {
        return Err(rejected("too many taskbars"));
    }
    let mut ids = BTreeSet::new();
    let mut snapshots = Vec::with_capacity(report.taskbars.len());
    let mut handles = Vec::with_capacity(report.taskbars.len());
    for record in &report.taskbars {
        if !ids.insert(record.id) {
            return Err(rejected("duplicate taskbar id"));
        }
        let key = TaskbarKey {
            generation: requested,
            id: TaskbarId(record.id),
        };
        let mut snapshot = TaskbarSnapshot {
            key,
            taskbar_rect: rect_from_wire(record.taskbar)?,
            clock_rect: None,
            monitor: MonitorId(record.monitor_id),
            monitor_rect: rect_from_wire(record.monitor)?,
            dpi: record.dpi,
            primary: record.primary,
            visibility: if record.visible {
                TaskbarVisibility::Visible
            } else {
                TaskbarVisibility::Hidden
            },
            strategy: if gate.needs_uia() {
                DiscoveryStrategy::UiAutomation
            } else {
                DiscoveryStrategy::NativeChildren
            },
        };
        snapshot.clock_rect = clock_rect(gate, &snapshot, requested, record.clock.as_ref())?;
        snapshots.push(snapshot);
        handles.push((key, record.hwnd));
    }
    Ok(Parsed {
        shell: report.shell.map(|s| ShellIdentity {
            pid: s.pid,
            started: s.started,
        }),
        snapshots,
        handles,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn report(generation: u64) -> serde_json::Value {
        json!({
            "schema": 2, "generation": generation,
            "shell": {"pid": 4242, "started": 133_000_000_000_000_000u64},
            "taskbars": [{
                "id": 1, "hwnd": 65_538, "taskbar": [-1920, 1032, 1920, 48],
                "monitor_id": 1, "monitor": [-1920, 0, 1920, 1080], "dpi": 144,
                "primary": false, "visible": true,
                "clock": {
                    "rect": [-120, 1032, 100, 48],
                    "adjacent": [[-20, 1032, 20, 48]],
                    "exclusions_complete": true
                }
            }]
        })
    }
    fn parse(
        value: &serde_json::Value,
        generation: u64,
        gate: Gate,
    ) -> Result<Parsed, DiscoveryFailure> {
        parse_report(
            value.to_string().as_bytes(),
            ShellGeneration(generation),
            gate,
        )
    }

    #[test]
    fn a_closed_gate_never_yields_a_clock_rectangle_even_for_a_perfect_candidate() {
        let parsed = parse(&report(7), 7, Gate::Closed).unwrap();
        assert_eq!(parsed.snapshots.len(), 1);
        assert_eq!(parsed.snapshots[0].clock_rect, None);
        assert_eq!(
            parsed.snapshots[0].strategy,
            DiscoveryStrategy::NativeChildren
        );
        assert_eq!(
            parsed.shell,
            Some(ShellIdentity {
                pid: 4242,
                started: 133_000_000_000_000_000
            })
        );
        assert_eq!(parsed.handles, vec![(parsed.snapshots[0].key, 65_538)]);
    }

    #[test]
    fn the_experimental_gate_still_requires_geometric_validation() {
        let parsed = parse(&report(7), 7, Gate::Experimental).unwrap();
        assert_eq!(
            parsed.snapshots[0].clock_rect,
            Some(PhysicalRect::new(-120, 1032, 100, 48).unwrap())
        );
        // Overlap with a neighbour, an incomplete inventory, or a rectangle
        // outside the taskbar each remove the rectangle rather than guessing.
        for (field, value) in [
            ("adjacent", json!([[-40, 1032, 40, 48]])),
            ("exclusions_complete", json!(false)),
            ("rect", json!([-120, 1000, 100, 48])),
        ] {
            let mut bad = report(7);
            bad["taskbars"][0]["clock"][field] = value;
            let parsed = parse(&bad, 7, Gate::Experimental).unwrap();
            assert_eq!(parsed.snapshots[0].clock_rect, None, "{field}");
        }
    }

    #[test]
    fn stale_and_foreign_envelopes_are_rejected() {
        assert!(matches!(
            parse(&report(6), 7, Gate::Closed),
            Err(DiscoveryFailure::Rejected(_))
        ));
        let mut other_schema = report(7);
        other_schema["schema"] = json!(1);
        assert!(parse(&other_schema, 7, Gate::Closed).is_err());
        assert!(parse_report(b"", ShellGeneration(7), Gate::Closed).is_err());
        assert!(parse_report(b"{not json", ShellGeneration(7), Gate::Closed).is_err());
        assert!(
            parse_report(
                &vec![b' '; (MAX_OUTPUT_BYTES + 1) as usize],
                ShellGeneration(7),
                Gate::Closed
            )
            .is_err()
        );
    }

    #[test]
    fn malformed_content_is_rejected_atomically() {
        type Mutation = Box<dyn Fn(&mut serde_json::Value)>;
        let cases: Vec<Mutation> = vec![
            Box::new(|v| v["taskbars"][0]["taskbar"] = json!([0, 0, 0, 10])),
            Box::new(|v| v["taskbars"][0]["taskbar"] = json!([0, 0, 10])),
            Box::new(|v| v["taskbars"][0]["monitor"] = json!([i64::MAX, 0, 10, 10])),
            Box::new(|v| v["taskbars"][0]["dpi"] = json!(-1)),
            Box::new(|v| v["unexpected"] = json!(true)),
            Box::new(|v| {
                let first = v["taskbars"][0].clone();
                v["taskbars"].as_array_mut().unwrap().push(first);
            }),
            Box::new(|v| {
                let first = v["taskbars"][0].clone();
                let list = v["taskbars"].as_array_mut().unwrap();
                for id in 2..=(MAX_TASKBARS as u64 + 1) {
                    let mut next = first.clone();
                    next["id"] = json!(id);
                    list.push(next);
                }
            }),
        ];
        for (index, mutate) in cases.iter().enumerate() {
            let mut value = report(7);
            mutate(&mut value);
            assert!(parse(&value, 7, Gate::Closed).is_err(), "case {index}");
        }
    }

    #[test]
    fn no_taskbars_and_no_shell_are_representable() {
        let value = json!({"schema": 2, "generation": 3, "shell": null, "taskbars": []});
        let parsed = parse(&value, 3, Gate::Closed).unwrap();
        assert!(parsed.snapshots.is_empty());
        assert_eq!(parsed.shell, None);
    }
}
