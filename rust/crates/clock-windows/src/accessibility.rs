//! Minimal UI Automation provider for the clock overlay (R09, Windows only).
//!
//! The overlay is a plain HWND whose pixels no assistive technology can read, so
//! it exposes one element:
//!
//! * **Name** = the displayed clock text, in reading order (`accessible_name`).
//! * **Control type** = Button, with the **Invoke** pattern as the single action.
//!   Invoke runs the same calendar action as a left click.
//! * Not keyboard focusable: the overlay is non-activating by design, so
//!   keyboard users reach the app through the tray icon and its menu instead.
//!
//! There is deliberately **no value pattern and no event raising**. A clock that
//! changes every second would flood a screen reader with live announcements; with
//! only a pull-model Name, a screen reader reads the current time when the user
//! asks for it and never speaks unprompted.
//!
//! Threading: UIA calls provider methods on a thread of its choosing, never the
//! UI thread. The provider therefore holds only a mutex-protected string, an
//! atomic liveness flag and a window id, and `Invoke` merely queues an
//! interaction (a thread-safe post to the UI thread) and returns. A client that
//! keeps its element after the overlay is destroyed gets
//! `UIA_E_ELEMENTNOTAVAILABLE`, and `UiaDisconnectProvider` is called first so
//! UIA drops its references.
use crate::interaction::{Interaction, accessible_name, post_interaction};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::Variant::VARIANT,
        UI::Accessibility::*,
    },
    core::{Error, HRESULT, IUnknown, IUnknownImpl, Result, implement},
};

const AUTOMATION_ID: &str = "Tickbar.Clock";
const HELP_TEXT: &str = "Opens the calendar and notifications";

/// State shared with whatever thread UIA calls on.
struct Shared {
    window: u64,
    name: Mutex<String>,
    alive: AtomicBool,
}
impl Shared {
    fn name(&self) -> String {
        self.name
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
    fn ensure_alive(&self) -> Result<()> {
        if self.alive.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(Error::from_hresult(HRESULT(
                UIA_E_ELEMENTNOTAVAILABLE as i32,
            )))
        }
    }
}

#[implement(IRawElementProviderSimple, IInvokeProvider)]
struct ClockProvider {
    shared: Arc<Shared>,
}

// The trait's method names and the UIA constants matched below are fixed by Windows.
#[allow(non_snake_case, non_upper_case_globals)]
impl IRawElementProviderSimple_Impl for ClockProvider_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider)
    }

    fn GetPatternProvider(&self, pattern: UIA_PATTERN_ID) -> Result<IUnknown> {
        self.shared.ensure_alive()?;
        if pattern == UIA_InvokePatternId {
            let invoke: IInvokeProvider = self.to_interface();
            Ok(invoke.into())
        } else {
            // S_OK with a null pointer: "this pattern is not supported".
            Err(Error::empty())
        }
    }

    fn GetPropertyValue(&self, property: UIA_PROPERTY_ID) -> Result<VARIANT> {
        self.shared.ensure_alive()?;
        Ok(match property {
            UIA_NamePropertyId => VARIANT::from(self.shared.name().as_str()),
            UIA_AutomationIdPropertyId => VARIANT::from(AUTOMATION_ID),
            UIA_HelpTextPropertyId => VARIANT::from(HELP_TEXT),
            UIA_ControlTypePropertyId => VARIANT::from(UIA_ButtonControlTypeId.0),
            UIA_IsControlElementPropertyId
            | UIA_IsContentElementPropertyId
            | UIA_IsEnabledPropertyId => VARIANT::from(true),
            UIA_IsKeyboardFocusablePropertyId | UIA_HasKeyboardFocusPropertyId => {
                VARIANT::from(false)
            }
            // Everything else (bounds, process, class, visibility) comes from the
            // window itself through the host provider.
            _ => VARIANT::default(),
        })
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        self.shared.ensure_alive()?;
        // SAFETY: plain query on a window id this provider was created for; a
        // destroyed window returns an error.
        unsafe { UiaHostProviderFromHwnd(HWND(self.shared.window as usize as *mut _)) }
    }
}

#[allow(non_snake_case)]
impl IInvokeProvider_Impl for ClockProvider_Impl {
    fn Invoke(&self) -> Result<()> {
        self.shared.ensure_alive()?;
        // Not on the UI thread: queue and return. The loop performs the single
        // calendar action, exactly as for a mouse click.
        post_interaction(Interaction::Activate {
            window: self.shared.window,
        });
        Ok(())
    }
}

/// The overlay's accessibility object. Created and dropped on the UI thread.
pub struct ClockAccessible {
    shared: Arc<Shared>,
    provider: IRawElementProviderSimple,
}
impl ClockAccessible {
    pub fn new(window: HWND) -> Self {
        let shared = Arc::new(Shared {
            window: window.0 as usize as u64,
            name: Mutex::new(String::new()),
            alive: AtomicBool::new(true),
        });
        let provider = ClockProvider {
            shared: shared.clone(),
        }
        .into();
        Self { shared, provider }
    }

    /// Record the displayed text. No event is raised, by design.
    pub fn set_text(&self, lines: &[String]) {
        *self
            .shared
            .name
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = accessible_name(lines);
    }

    /// Detach from UIA. Idempotent; called before the window is destroyed.
    pub fn disconnect(&self) {
        if self.shared.alive.swap(false, Ordering::AcqRel) {
            // SAFETY: tells UIA to release its references to this provider.
            unsafe {
                let _ = UiaDisconnectProvider(&self.provider);
            }
        }
    }

    pub fn provider(&self) -> IRawElementProviderSimple {
        self.provider.clone()
    }
}
impl Drop for ClockAccessible {
    fn drop(&mut self) {
        self.disconnect();
    }
}

/// Answer `WM_GETOBJECT` for the window's root object with `provider`.
pub fn return_object(
    provider: &IRawElementProviderSimple,
    window: HWND,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: `provider` is a live COM object; UIA takes its own reference.
    unsafe { UiaReturnRawElementProvider(window, wparam, lparam, provider) }
}

/// `WM_GETOBJECT` object id for the window's root element.
pub fn is_root_object(lparam: LPARAM) -> bool {
    lparam.0 as i32 == UiaRootObjectId
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interaction::take_interactions;
    use windows::core::Interface;

    fn provider(window: usize) -> ClockAccessible {
        ClockAccessible::new(HWND(window as *mut _))
    }
    fn property(provider: &IRawElementProviderSimple, id: UIA_PROPERTY_ID) -> Result<VARIANT> {
        // SAFETY: calls an in-process COM object on its own thread.
        unsafe { provider.GetPropertyValue(id) }
    }

    #[test]
    fn the_element_is_a_non_focusable_button_named_by_the_displayed_text() {
        let accessible = provider(0x1234);
        accessible.set_text(&["12:34".to_string(), "14/10/2026".to_string()]);
        let element = accessible.provider();
        // SAFETY: as above.
        assert_eq!(
            unsafe { element.ProviderOptions() },
            Ok(ProviderOptions_ServerSideProvider)
        );
        assert_eq!(accessible.shared.name(), "12:34, 14/10/2026");
        let control_type = property(&element, UIA_ControlTypePropertyId).unwrap();
        assert_eq!(i32::try_from(&control_type), Ok(UIA_ButtonControlTypeId.0));
        for id in [
            UIA_IsKeyboardFocusablePropertyId,
            UIA_HasKeyboardFocusPropertyId,
        ] {
            let value = property(&element, id).unwrap();
            assert_eq!(bool::try_from(&value), Ok(false));
        }
        // A new minute replaces the name; no event machinery exists to flood a reader.
        accessible.set_text(&["12:35".to_string()]);
        assert_eq!(accessible.shared.name(), "12:35");
    }

    #[test]
    fn invoke_is_the_only_pattern_and_queues_one_activation_for_the_ui_thread() {
        let accessible = provider(0x4321);
        let element = accessible.provider();
        // SAFETY: as above.
        unsafe {
            assert!(element.GetPatternProvider(UIA_ValuePatternId).is_err());
            assert!(element.GetPatternProvider(UIA_TogglePatternId).is_err());
            let invoke: IInvokeProvider = element
                .GetPatternProvider(UIA_InvokePatternId)
                .unwrap()
                .cast()
                .unwrap();
            invoke.Invoke().unwrap();
        }
        let queued = take_interactions();
        assert_eq!(queued, vec![Interaction::Activate { window: 0x4321 }]);
    }

    #[test]
    fn a_disconnected_provider_reports_the_element_unavailable_and_acts_on_nothing() {
        let accessible = provider(0x5555);
        let element = accessible.provider();
        // SAFETY: as above.
        let invoke: IInvokeProvider = unsafe {
            element
                .GetPatternProvider(UIA_InvokePatternId)
                .unwrap()
                .cast()
                .unwrap()
        };
        accessible.disconnect();
        accessible.disconnect(); // idempotent
        let gone = Error::from_hresult(HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32));
        let name = property(&element, UIA_NamePropertyId);
        assert_eq!(name.err().map(|e| e.code()), Some(gone.code()));
        // SAFETY: as above.
        assert_eq!(unsafe { invoke.Invoke() }.unwrap_err().code(), gone.code());
        assert!(take_interactions().is_empty(), "no action after disconnect");
    }
}
