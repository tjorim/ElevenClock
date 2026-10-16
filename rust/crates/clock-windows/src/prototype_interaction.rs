//! R03 helper-only UIA experiment. No COM object or HWND leaves this process.
use crate::overlay::ExperimentSlot;
use crate::taskbar::{
    candidate::{self, CandidateElement},
    geometry::validate_candidate,
    probe,
};
use clock_core::ShellGeneration;
use windows::{
    Win32::{Foundation::POINT, System::Com::*, UI::Accessibility::*},
    core::{Interface, Result},
};

/// Split (`OmniButtonLeft`) and merged (`OmniButton`) clock buttons; see
/// [`candidate`] for which one a desktop exposes.
const CLOCK_CLASSES: [&str; 2] = ["SystemTray.OmniButtonLeft", "SystemTray.OmniButton"];

struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}
fn rejected() -> windows::core::Error {
    windows::core::Error::new(
        windows::core::HRESULT(0x80004005u32 as i32),
        "Clock identity/hit-target experiment failed closed",
    )
}

unsafe fn clock_at(
    automation: &IUIAutomation,
    point: POINT,
    slot: &ExperimentSlot,
) -> Result<IUIAutomationElement> {
    // Walk the independent point hit up to the clock button, never find an
    // overlay by its label or invoke an element obtained from the overlay HWND.
    unsafe {
        let walker = automation.RawViewWalker()?;
        let mut node = automation.ElementFromPoint(point)?;
        let mut ancestry = Vec::new();
        for _ in 0..16 {
            let class = node.CurrentClassName()?.to_string();
            if CLOCK_CLASSES.contains(&class.as_str())
                && node.CurrentControlType()? == UIA_ButtonControlTypeId
                && node.CurrentProcessId()? as u32 == slot.shell_pid
                && !node.CurrentIsOffscreen()?.as_bool()
                && probe::physical_rect(node.CurrentBoundingRectangle()?) == Some(slot.clock)
            {
                return Ok(node);
            }
            ancestry.push(format!("{class}: {:?}", node.CurrentBoundingRectangle()));
            node = walker.GetParentElement(&node).map_err(|error| {
                windows::core::Error::new(
                    windows::core::HRESULT(0x80004005u32 as i32),
                    format!("Clock hit ancestry {ancestry:?}; parent unavailable: {error}"),
                )
            })?;
        }
    }
    Err(rejected())
}

/// Dense independent UIA hit sampling is experimental evidence on this exact
/// desktop, not authorization for R02's production selector.
pub fn discover(generation: u64, all: bool) -> Result<Vec<ExperimentSlot>> {
    let records = probe::collect(ShellGeneration(generation)).map_err(|error| {
        windows::core::Error::new(
            windows::core::HRESULT(0x80004005u32 as i32),
            format!("R03 native/UIA inventory: {error}"),
        )
    })?;
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
    }
    let _apartment = Apartment;
    let automation: IUIAutomation =
        unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)? };
    let mut slots = Vec::new();
    for record in records {
        let s = &record.snapshot;
        if !all && !s.primary {
            continue;
        }
        if record.uia.truncated || !record.uia.failures.is_empty() {
            return Err(rejected());
        }
        // The production selector decides the clock (split or merged layout);
        // the spike then only adds its independent hit sampling.
        let elements: Vec<_> = record
            .uia
            .elements
            .iter()
            .map(|e| CandidateElement {
                parent: e.parent,
                automation_id: e.automation_id.clone(),
                class: e.class.clone(),
                control_type: e.control_type,
                process_id: e.process_id,
                offscreen: e.offscreen,
                rect: e.rect,
            })
            .collect();
        let selected = candidate::select(
            &elements,
            record.shell_pid,
            !record.native_inventory_incomplete,
        )
        .ok_or_else(rejected)?;
        if !selected.exclusions_complete {
            return Err(rejected());
        }
        let clock = selected.rect;
        let (index, candidate) = record
            .uia
            .elements
            .iter()
            .enumerate()
            .find(|(_, e)| CLOCK_CLASSES.contains(&e.class.as_str()) && e.rect == Some(clock))
            .ok_or_else(rejected)?;
        let neighbors: Vec<_> = record
            .uia
            .elements
            .iter()
            .enumerate()
            .filter(|(i, e)| {
                *i != index
                    && e.parent == candidate.parent
                    && e.control_type == 50000
                    && !e.offscreen
            })
            .collect();
        let adjacent: Vec<_> = neighbors.iter().filter_map(|(_, e)| e.rect).collect();
        validate_candidate(s, ShellGeneration(generation), clock, &adjacent, true)
            .map_err(|_| rejected())?;
        let slot = ExperimentSlot {
            clock,
            taskbar: s.taskbar_rect,
            monitor: s.monitor_rect,
            dpi: s.dpi,
            shell_pid: record.shell_pid,
            shell_started: record.shell_started,
            primary: s.primary,
        };
        // Four-pixel interior grid, including the last interior row/column.
        // A bounded process deadline handles an expensive or blocked provider.
        for (x, y) in crate::overlay::clock_hit_points(clock).ok_or_else(rejected)? {
            unsafe {
                clock_at(&automation, POINT { x, y }, &slot).map_err(|error| {
                    windows::core::Error::new(
                        windows::core::HRESULT(0x80004005u32 as i32),
                        format!("R03 clock hit at ({x}, {y}): {error}"),
                    )
                })?;
            }
        }
        // Independent neighbor center hits must resolve to that exact control.
        for (_, e) in neighbors {
            let r = e.rect.ok_or_else(rejected)?;
            let hit = unsafe {
                automation.ElementFromPoint(POINT {
                    x: r.left() + r.width() / 2,
                    y: r.top() + r.height() / 2,
                })?
            };
            let walker = unsafe { automation.RawViewWalker()? };
            let mut hit = hit;
            let mut matched = false;
            for _ in 0..16 {
                if unsafe { hit.CurrentClassName()? } == e.class.as_str()
                    && unsafe { hit.CurrentProcessId()? } == e.process_id
                    && probe::physical_rect(unsafe { hit.CurrentBoundingRectangle()? }) == Some(r)
                {
                    matched = true;
                    break;
                }
                match unsafe { walker.GetParentElement(&hit) } {
                    Ok(parent) => hit = parent,
                    Err(_) => break,
                }
            }
            if !matched {
                return Err(rejected());
            }
        }
        slots.push(slot);
    }
    if slots.is_empty() {
        return Err(rejected());
    }
    Ok(slots)
}

/// UIA is attempted once. An Invoke error is ambiguous and MUST NOT trigger a
/// second action. Only absence of the pattern permits deliberate Win+N testing.
pub fn invoke(slot: &ExperimentSlot, fallback: bool, keyboard_only: bool) -> Result<&'static str> {
    crate::overlay::check_slot(slot)?;
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
    }
    let _apartment = Apartment;
    let automation: IUIAutomation =
        unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)? };
    let node = unsafe {
        clock_at(
            &automation,
            POINT {
                x: slot.clock.left() + slot.clock.width() / 2,
                y: slot.clock.top() + slot.clock.height() / 2,
            },
            slot,
        )?
    };
    crate::overlay::check_slot(slot)?;
    if keyboard_only {
        // A separately selected experiment, never a retry after UIA success or
        // failure. Identity is still independently checked before input.
        win_n()?;
        return Ok("win_n_once");
    }
    // Preserve S_OK + null (pattern unavailable), which the generated binding
    // reports as E_POINTER. Provider errors never authorize keyboard fallback.
    let mut pattern = std::ptr::null_mut();
    unsafe {
        (Interface::vtable(&node).GetCurrentPattern)(
            node.as_raw(),
            UIA_InvokePatternId,
            &mut pattern,
        )
        .ok()?;
    }
    if pattern.is_null() {
        if !fallback {
            return Err(rejected());
        }
        win_n()?;
        return Ok("win_n_once");
    }
    let pattern = unsafe { windows::core::IUnknown::from_raw(pattern) };
    let invoke: IUIAutomationInvokePattern = pattern.cast()?;
    unsafe {
        invoke.Invoke()?;
    }
    Ok("uia_invoke_once")
}

fn win_n() -> Result<()> {
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    // Do not release modifiers owned by the user. A single batch, no retries.
    for key in [
        VK_LWIN,
        VK_RWIN,
        VK_CONTROL,
        VK_SHIFT,
        VK_MENU,
        VIRTUAL_KEY(0x4e),
    ] {
        if unsafe { GetAsyncKeyState(i32::from(key.0)) } < 0 {
            return Err(rejected());
        }
    }
    let input = |key, flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                dwFlags: flags,
                ..Default::default()
            },
        },
    };
    let keys = [
        input(VK_LWIN, KEYBD_EVENT_FLAGS(0)),
        input(VIRTUAL_KEY(0x4e), KEYBD_EVENT_FLAGS(0)),
        input(VIRTUAL_KEY(0x4e), KEYEVENTF_KEYUP),
        input(VK_LWIN, KEYEVENTF_KEYUP),
    ];
    let count = unsafe { SendInput(&keys, std::mem::size_of::<INPUT>() as i32) };
    if count != 4 {
        // Release only keys whose injected key-down was accepted; no repeated
        // shortcut or click replay. Partial cleanup is reported as failure.
        let mut cleanup = Vec::new();
        if count >= 2 {
            cleanup.push(input(VIRTUAL_KEY(0x4e), KEYEVENTF_KEYUP));
        }
        if count >= 1 {
            cleanup.push(input(VK_LWIN, KEYEVENTF_KEYUP));
        }
        if !cleanup.is_empty() {
            unsafe {
                SendInput(&cleanup, std::mem::size_of::<INPUT>() as i32);
            }
        }
        return Err(rejected());
    }
    Ok(())
}
