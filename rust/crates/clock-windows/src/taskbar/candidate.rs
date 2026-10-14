//! The R02 *experimental* clock candidate, as a pure function over a UIA
//! inventory (experimental: P1 is NO-GO, see #23).
//!
//! This reproduces what the hosted captures recorded and R03's spike used: a
//! visible XAML `SystemTray.OmniButtonLeft` button owned by the shell whose
//! children expose `TimeInnerTextBlock` and `DateInnerTextBlock`, with the bell
//! (`OmniButtonRight`) and Show Desktop buttons as neighbours. It uses only
//! locale-independent properties: never a localized Name, an ordinal or an offset.
//! Passing it proves nothing about hit targets on untested configurations, which
//! is why it stays experimental until P1 is a GO.
use clock_core::PhysicalRect;

/// The inventory fields the selection needs; native types convert into this.
#[derive(Clone, Debug)]
pub struct CandidateElement {
    pub parent: Option<usize>,
    pub automation_id: String,
    pub class: String,
    pub control_type: i32,
    pub process_id: i32,
    pub offscreen: bool,
    pub rect: Option<PhysicalRect>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Candidate {
    pub rect: PhysicalRect,
    pub adjacent: Vec<PhysicalRect>,
    /// Inventory was complete: not truncated, no provider failures, and every
    /// sibling button had a measurable rectangle.
    pub exclusions_complete: bool,
}

const BUTTON: i32 = 50000;

pub fn select(
    elements: &[CandidateElement],
    shell_pid: u32,
    inventory_complete: bool,
) -> Option<Candidate> {
    let has_child = |parent: usize, id: &str| {
        elements
            .iter()
            .any(|c| c.parent == Some(parent) && c.automation_id == id)
    };
    let mut found = elements.iter().enumerate().filter(|(index, e)| {
        e.class == "SystemTray.OmniButtonLeft"
            && e.control_type == BUTTON
            && !e.offscreen
            && u32::try_from(e.process_id) == Ok(shell_pid)
            && has_child(*index, "TimeInnerTextBlock")
            && has_child(*index, "DateInnerTextBlock")
    });
    let (index, candidate) = found.next()?;
    if found.next().is_some() {
        return None; // Ambiguous: never pick one.
    }
    let rect = candidate.rect?;
    let neighbours: Vec<_> = elements
        .iter()
        .enumerate()
        .filter(|(i, e)| {
            *i != index && e.parent == candidate.parent && e.control_type == BUTTON && !e.offscreen
        })
        .map(|(_, e)| e)
        .collect();
    let has_class = |class: &str| neighbours.iter().any(|e| e.class == class);
    if !has_class("SystemTray.OmniButtonRight") || !has_class("SystemTray.ShowDesktopButton") {
        return None; // The bell/Show Desktop anchors must be present to trust adjacency.
    }
    let measurable = neighbours.iter().all(|e| e.rect.is_some());
    Some(Candidate {
        rect,
        adjacent: neighbours.iter().filter_map(|e| e.rect).collect(),
        exclusions_complete: inventory_complete && measurable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(l: i32, t: i32, w: i32, h: i32) -> Option<PhysicalRect> {
        Some(PhysicalRect::new(l, t, w, h).unwrap())
    }
    fn element(
        parent: Option<usize>,
        id: &str,
        class: &str,
        rect: Option<PhysicalRect>,
    ) -> CandidateElement {
        CandidateElement {
            parent,
            automation_id: id.into(),
            class: class.into(),
            control_type: BUTTON,
            process_id: 1234,
            offscreen: false,
            rect,
        }
    }
    /// The hosted capture: clock [910,720,984,768], bell [984..1012], desktop [1012..1024].
    fn tray() -> Vec<CandidateElement> {
        vec![
            element(None, "", "Shell_TrayWnd", r(0, 720, 1024, 48)), // 0
            element(Some(0), "", "SystemTray.NormalButton", r(900, 720, 10, 48)), // 1 (not a neighbour class of interest)
            element(
                Some(0),
                "SystemTrayIcon",
                "SystemTray.OmniButtonLeft",
                r(910, 720, 74, 48),
            ), // 2
            element(
                Some(2),
                "TimeInnerTextBlock",
                "TextBlock",
                r(915, 724, 60, 20),
            ),
            element(
                Some(2),
                "DateInnerTextBlock",
                "TextBlock",
                r(915, 744, 60, 20),
            ),
            element(
                Some(0),
                "SystemTrayIcon",
                "SystemTray.OmniButtonRight",
                r(984, 720, 28, 48),
            ),
            element(
                Some(0),
                "ShowDesktop",
                "SystemTray.ShowDesktopButton",
                r(1012, 720, 12, 48),
            ),
        ]
    }

    #[test]
    fn selects_the_recorded_candidate_with_complete_adjacency() {
        let found = select(&tray(), 1234, true).unwrap();
        assert_eq!(Some(found.rect), r(910, 720, 74, 48));
        assert!(found.exclusions_complete);
        assert!(found.adjacent.contains(&r(984, 720, 28, 48).unwrap()));
        assert!(found.adjacent.contains(&r(1012, 720, 12, 48).unwrap()));
    }

    #[test]
    fn refuses_ambiguous_missing_or_foreign_candidates() {
        // Another process' lookalike.
        assert_eq!(select(&tray(), 999, true), None);
        // Missing time or date children.
        let mut no_date = tray();
        no_date.remove(4);
        assert_eq!(select(&no_date, 1234, true), None);
        // Offscreen.
        let mut hidden = tray();
        hidden[2].offscreen = true;
        assert_eq!(select(&hidden, 1234, true), None);
        // Two matching buttons are ambiguous, never "pick the first".
        let mut twin = tray();
        let len = twin.len();
        twin.push(element(
            Some(0),
            "SystemTrayIcon",
            "SystemTray.OmniButtonLeft",
            r(0, 0, 10, 10),
        ));
        twin.push(element(Some(len), "TimeInnerTextBlock", "TextBlock", None));
        twin.push(element(Some(len), "DateInnerTextBlock", "TextBlock", None));
        assert_eq!(select(&twin, 1234, true), None);
        // Bell or Show Desktop missing: adjacency cannot be trusted.
        let mut no_bell = tray();
        no_bell.remove(5);
        assert_eq!(select(&no_bell, 1234, true), None);
        let mut no_desktop = tray();
        no_desktop.remove(6);
        assert_eq!(select(&no_desktop, 1234, true), None);
        // No measured rectangle.
        let mut unmeasured = tray();
        unmeasured[2].rect = None;
        assert_eq!(select(&unmeasured, 1234, true), None);
    }

    #[test]
    fn incomplete_inventory_or_unmeasured_neighbours_mark_exclusions_incomplete() {
        assert!(!select(&tray(), 1234, false).unwrap().exclusions_complete);
        let mut unmeasured = tray();
        unmeasured[5].rect = None;
        assert!(!select(&unmeasured, 1234, true).unwrap().exclusions_complete);
    }
}
