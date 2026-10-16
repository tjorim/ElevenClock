//! The R02 *experimental* clock candidate, as a pure function over a UIA
//! inventory (experimental: P1 is NO-GO, see #21).
//!
//! This reproduces what the hosted captures recorded and R03's spike used: a
//! visible XAML `SystemTray.OmniButtonLeft` button owned by the shell whose
//! children expose `TimeInnerTextBlock` and `DateInnerTextBlock`, with the bell
//! (`OmniButtonRight`) and Show Desktop buttons as neighbours. It uses only
//! locale-independent properties: never a localized Name, an ordinal or an offset.
//! Passing it proves nothing about hit targets on untested configurations, which
//! is why it stays experimental until P1 is a GO.
//!
//! A stock x64 desktop (build 26200.9457, #21) instead exposes one merged
//! `SystemTray.OmniButton` with the same time/date children and Show Desktop as
//! its neighbour; on a secondary taskbar the same button stands alone. See
//! [`Layout`].
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

/// Recorded tray layouts. Hosted captures split the clock (`OmniButtonLeft`)
/// from the bell (`OmniButtonRight`); stock x64 build 26200.9457 has a single
/// `OmniButton` holding only the time and date, next to Show Desktop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Layout {
    SplitClock,
    MergedClock,
}

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
    // A merged clock button may later host other glyphs; accept it only while
    // its visible content is exactly the time and date text.
    let only_clock_text = |parent: usize| {
        elements
            .iter()
            .filter(|c| c.parent == Some(parent) && !c.offscreen)
            .all(|c| {
                matches!(
                    c.automation_id.as_str(),
                    "TimeInnerTextBlock" | "DateInnerTextBlock"
                )
            })
    };
    let mut found = elements.iter().enumerate().filter_map(|(index, e)| {
        let layout = match e.class.as_str() {
            "SystemTray.OmniButtonLeft" => Layout::SplitClock,
            "SystemTray.OmniButton" => Layout::MergedClock,
            _ => return None,
        };
        (e.control_type == BUTTON
            && !e.offscreen
            && u32::try_from(e.process_id) == Ok(shell_pid)
            && has_child(index, "TimeInnerTextBlock")
            && has_child(index, "DateInnerTextBlock")
            && (layout == Layout::SplitClock || only_clock_text(index)))
        .then_some((index, e, layout))
    });
    let (index, candidate, layout) = found.next()?;
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
    // Split clocks require a separate bell. Show Desktop anchors primary
    // taskbars only: secondary taskbars omit it in both recorded layouts
    // (including the split clock captured on x64 build 26300.9711).
    let secondary = elements
        .iter()
        .any(|e| e.parent.is_none() && e.class == "Shell_SecondaryTrayWnd");
    let needs_show_desktop = !secondary;
    if (layout == Layout::SplitClock && !has_class("SystemTray.OmniButtonRight"))
        || (needs_show_desktop && !has_class("SystemTray.ShowDesktopButton"))
    {
        return None;
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

    /// Stock x64 build 26200.9457 at 120 DPI (#21): taskbar [0,1140,1920,1200];
    /// a merged clock button [1813..1900] then Show Desktop [1905..1920].
    fn merged_tray() -> Vec<CandidateElement> {
        vec![
            element(None, "", "Shell_TrayWnd", r(0, 1140, 1920, 60)), // 0
            element(
                Some(0),
                "SystemTrayIcon",
                "SystemTray.OmniButtonRight",
                r(1773, 1140, 30, 60),
            ), // 1
            element(
                Some(0),
                "SystemTrayIcon",
                "SystemTray.OmniButton",
                r(1813, 1140, 87, 60),
            ), // 2
            element(
                Some(2),
                "TimeInnerTextBlock",
                "TextBlock",
                r(1818, 1149, 78, 20),
            ),
            element(
                Some(2),
                "DateInnerTextBlock",
                "TextBlock",
                r(1818, 1169, 78, 20),
            ),
            element(
                Some(0),
                "SystemTrayIcon",
                "SystemTray.ShowDesktopButton",
                r(1905, 1140, 15, 60),
            ), // 5
        ]
    }

    #[test]
    fn selects_a_merged_clock_button_next_to_show_desktop() {
        let found = select(&merged_tray(), 1234, true).unwrap();
        assert_eq!(Some(found.rect), r(1813, 1140, 87, 60));
        assert!(found.exclusions_complete);
        assert!(found.adjacent.contains(&r(1905, 1140, 15, 60).unwrap()));
        // A merged button has no separate bell to anchor on.
        let mut no_neighbour_bell = merged_tray();
        no_neighbour_bell[1].class = "SystemTray.NormalButton".into();
        assert!(select(&no_neighbour_bell, 1234, true).is_some());
    }

    #[test]
    fn refuses_merged_buttons_with_other_content_or_missing_anchor() {
        // A visible glyph (for example a bell) inside the merged button.
        let mut with_glyph = merged_tray();
        with_glyph.push(element(
            Some(2),
            "InnerTextBlock",
            "TextBlock",
            r(1880, 1160, 20, 20),
        ));
        assert_eq!(select(&with_glyph, 1234, true), None);
        // The same glyph offscreen does not count as content.
        with_glyph.last_mut().unwrap().offscreen = true;
        assert!(select(&with_glyph, 1234, true).is_some());
        let mut no_desktop = merged_tray();
        no_desktop.remove(5);
        assert_eq!(select(&no_desktop, 1234, true), None);
        // Split and merged candidates together are ambiguous.
        let mut both = merged_tray();
        let left = both.len();
        both.push(element(
            Some(0),
            "SystemTrayIcon",
            "SystemTray.OmniButtonLeft",
            r(1700, 1140, 60, 60),
        ));
        both.push(element(Some(left), "TimeInnerTextBlock", "TextBlock", None));
        both.push(element(Some(left), "DateInnerTextBlock", "TextBlock", None));
        assert_eq!(select(&both, 1234, true), None);
    }

    /// Same build, second 96-DPI display (#21): the secondary taskbar's tray holds
    /// only the merged clock [1970..2040]; there is no Show Desktop button.
    fn secondary_tray() -> Vec<CandidateElement> {
        vec![
            element(None, "", "Shell_SecondaryTrayWnd", r(1024, 720, 1024, 48)), // 0
            element(
                Some(0),
                "Appid: MSEdge",
                "Taskbar.TaskListButtonAutomationPeer",
                r(1500, 720, 44, 48),
            ), // 1
            element(
                Some(0),
                "SystemTrayIcon",
                "SystemTray.OmniButton",
                r(1970, 720, 70, 48),
            ), // 2
            element(
                Some(2),
                "TimeInnerTextBlock",
                "TextBlock",
                r(1974, 727, 63, 16),
            ),
            element(
                Some(2),
                "DateInnerTextBlock",
                "TextBlock",
                r(1974, 743, 63, 16),
            ),
        ]
    }

    #[test]
    fn secondary_taskbars_need_no_show_desktop_anchor() {
        let found = select(&secondary_tray(), 1234, true).unwrap();
        assert_eq!(Some(found.rect), r(1970, 720, 70, 48));
        assert!(found.exclusions_complete);
        // The same tray on a primary taskbar still lacks its required anchor.
        let mut as_primary = secondary_tray();
        as_primary[0].class = "Shell_TrayWnd".into();
        assert_eq!(select(&as_primary, 1234, true), None);
        // A split secondary clock still requires its separate bell.
        let mut split = secondary_tray();
        split[2].class = "SystemTray.OmniButtonLeft".into();
        assert_eq!(select(&split, 1234, true), None);
        // Other visible content inside the merged button is still refused.
        let mut with_glyph = secondary_tray();
        with_glyph.push(element(
            Some(2),
            "InnerTextBlock",
            "TextBlock",
            r(2020, 730, 16, 16),
        ));
        assert_eq!(select(&with_glyph, 1234, true), None);
    }

    #[test]
    fn split_secondary_clock_without_show_desktop_keeps_the_bell_excluded() {
        // Windows 11 26H2 x64 26300.9711, second 1280x720 display at 96 DPI.
        // Normalized from docs/windows-evidence/issue34-26300/auto-hide-off.jsonl.
        let mut elements = vec![
            element(None, "", "Shell_SecondaryTrayWnd", r(1280, 672, 1280, 48)),
            element(
                Some(0),
                "SystemTrayIcon",
                "SystemTray.OmniButtonLeft",
                r(2454, 672, 70, 48),
            ),
            element(
                Some(1),
                "TimeInnerTextBlock",
                "TextBlock",
                r(2458, 679, 63, 16),
            ),
            element(
                Some(1),
                "DateInnerTextBlock",
                "TextBlock",
                r(2458, 695, 63, 16),
            ),
            element(
                Some(0),
                "SystemTrayIcon",
                "SystemTray.OmniButtonRight",
                r(2528, 672, 24, 48),
            ),
        ];
        let found = select(&elements, 1234, true).unwrap();
        assert_eq!(Some(found.rect), r(2454, 672, 70, 48));
        assert_eq!(found.adjacent, vec![r(2528, 672, 24, 48).unwrap()]);
        assert!(found.exclusions_complete);
        assert!(!select(&elements, 1234, false).unwrap().exclusions_complete);
        elements[4].rect = None;
        assert!(!select(&elements, 1234, true).unwrap().exclusions_complete);
        elements[4].offscreen = true;
        assert!(select(&elements, 1234, true).is_none());
        elements[4].offscreen = false;
        elements[0].class = "Shell_TrayWnd".into();
        assert!(select(&elements, 1234, true).is_none());
    }

    #[test]
    fn incomplete_inventory_or_unmeasured_neighbours_mark_exclusions_incomplete() {
        assert!(!select(&tray(), 1234, false).unwrap().exclusions_complete);
        let mut unmeasured = tray();
        unmeasured[5].rect = None;
        assert!(!select(&unmeasured, 1234, true).unwrap().exclusions_complete);
    }
}
