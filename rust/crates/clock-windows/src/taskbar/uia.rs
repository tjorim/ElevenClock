//! Locale-independent property inventory, not a supported selector. Blocking
//! COM calls are allowed only in the development probe's isolated process.
use clock_core::PhysicalRect;
use std::time::Instant;
use windows::Win32::{Foundation::HWND, System::Com::*, UI::Accessibility::*};
use windows::core::Interface;

#[derive(Debug)]
pub struct Element {
    pub parent: Option<usize>,
    pub automation_id: String,
    pub class: String,
    pub framework: String,
    pub control_type: i32,
    pub process_id: i32,
    pub offscreen: bool,
    pub rect: Option<PhysicalRect>,
    pub elapsed_micros: u128,
}

#[derive(Debug, Default)]
pub struct Inventory {
    pub elements: Vec<Element>,
    pub failures: Vec<String>,
    pub truncated: bool,
    pub elapsed_micros: u128,
    pub longest_call_micros: u128,
    pub calls: Vec<CallTiming>,
}

#[derive(Debug)]
pub struct CallTiming {
    pub operation: &'static str,
    pub elapsed_micros: u128,
    pub succeeded: bool,
}

struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: initialization succeeded on this thread; COM objects were
        // dropped before this guard. The guard is never sent across threads.
        unsafe {
            CoUninitialize();
        }
    }
}

fn timed<T>(
    report: &mut Inventory,
    name: &'static str,
    call: impl FnOnce() -> windows::core::Result<T>,
) -> windows::core::Result<T> {
    let start = Instant::now();
    let result = call();
    let elapsed_micros = start.elapsed().as_micros();
    report.longest_call_micros = report.longest_call_micros.max(elapsed_micros);
    report.calls.push(CallTiming {
        operation: name,
        elapsed_micros,
        succeeded: result.is_ok(),
    });
    if let Err(error) = &result {
        report.failures.push(format!("{name}: {error}"));
    }
    result
}

// The generated non-optional return binding turns S_OK + null (normal end of
// traversal) into E_POINTER. Preserve that distinction using the COM ABI.
unsafe fn related(
    walker: &IUIAutomationTreeWalker,
    node: &IUIAutomationElement,
    child: bool,
) -> windows::core::Result<Option<IUIAutomationElement>> {
    let mut ptr = std::ptr::null_mut();
    let vtable = Interface::vtable(walker);
    let call = if child {
        vtable.GetFirstChildElement
    } else {
        vtable.GetNextSiblingElement
    };
    // SAFETY: live interfaces in one apartment; out pointer is writable. A
    // successful non-null output transfers exactly one owned COM reference.
    unsafe {
        call(walker.as_raw(), node.as_raw(), &mut ptr).ok()?;
    }
    Ok(if ptr.is_null() {
        None
    } else {
        Some(unsafe { IUIAutomationElement::from_raw(ptr) })
    })
}

/// All COM interfaces are created, used and released on this helper's main
/// thread. The parent owns cancellation by terminating the process, not COM.
pub fn inventory(hwnd: HWND) -> Inventory {
    let start = Instant::now();
    let mut report = Inventory::default();
    // SAFETY: this helper thread has no UI message loop or existing apartment.
    let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok();
    if let Err(error) = result {
        report.failures.push(format!("CoInitializeEx: {error}"));
        return report;
    }
    let _apartment = Apartment;
    // SAFETY: interfaces never escape this thread/process. No actions are invoked,
    // names read, or provider objects retained by application code.
    let result = unsafe {
        (|| -> windows::core::Result<()> {
            let automation: IUIAutomation = timed(&mut report, "CoCreateInstance", || {
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
            })?;
            let root = timed(&mut report, "ElementFromHandle", || {
                automation.ElementFromHandle(hwnd)
            })?;
            let walker = timed(&mut report, "RawViewWalker", || automation.RawViewWalker())?;
            let mut stack = vec![(root, None, 0usize)];
            while let Some((node, parent, depth)) = stack.pop() {
                if report.elements.len() >= 2048 || depth >= 64 {
                    report.truncated = true;
                    break;
                }
                let element_start = Instant::now();
                let index = report.elements.len();
                let automation_id = timed(&mut report, "CurrentAutomationId", || {
                    node.CurrentAutomationId()
                })?
                .to_string();
                let class =
                    timed(&mut report, "CurrentClassName", || node.CurrentClassName())?.to_string();
                let framework = timed(&mut report, "CurrentFrameworkId", || {
                    node.CurrentFrameworkId()
                })?
                .to_string();
                let control_type = timed(&mut report, "CurrentControlType", || {
                    node.CurrentControlType()
                })?
                .0;
                let process_id =
                    timed(&mut report, "CurrentProcessId", || node.CurrentProcessId())?;
                let offscreen = timed(&mut report, "CurrentIsOffscreen", || {
                    node.CurrentIsOffscreen()
                })?
                .as_bool();
                let rect = crate::taskbar::probe::physical_rect(timed(
                    &mut report,
                    "CurrentBoundingRectangle",
                    || node.CurrentBoundingRectangle(),
                )?);
                report.elements.push(Element {
                    parent,
                    automation_id,
                    class,
                    framework,
                    control_type,
                    process_id,
                    offscreen,
                    rect,
                    elapsed_micros: element_start.elapsed().as_micros(),
                });
                if parent.is_some()
                    && let Some(sibling) = timed(&mut report, "GetNextSiblingElement", || {
                        related(&walker, &node, false)
                    })?
                {
                    stack.push((sibling, parent, depth));
                }
                if let Some(child) = timed(&mut report, "GetFirstChildElement", || {
                    related(&walker, &node, true)
                })? {
                    stack.push((child, Some(index), depth + 1));
                }
            }
            Ok(())
        })()
    };
    if let Err(error) = result {
        report
            .failures
            .push(format!("Inventory incomplete: {error}"));
    }
    report.elapsed_micros = start.elapsed().as_micros();
    report
}
