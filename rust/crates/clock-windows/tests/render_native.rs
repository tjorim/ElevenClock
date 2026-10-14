//! Real DirectWrite/Direct2D checks against a hidden window. These run on Windows
//! CI; they do not replace the interactive DPI/theme screenshot matrix.
#![cfg(windows)]

use chrono::{FixedOffset, TimeZone};
use clock_core::{ClockText, Preferences};
use clock_windows::{
    render::{ClockSurface, Fit, PaintOutcome, PaintRequest, PixelSize, Renderer, WorstCase},
    theme::Palette,
};
use windows::{
    Win32::{
        Foundation::HWND,
        System::Threading::{
            GR_GDIOBJECTS, GR_USEROBJECTS, GetCurrentProcess, GetGuiResources,
            GetProcessHandleCount,
        },
        UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
        },
    },
    core::w,
};

/// A hidden, never-shown window of exact physical size standing in for an overlay.
struct Host(HWND);
impl Host {
    fn new(width: i32, height: i32) -> Self {
        // SAFETY: STATIC is a system class; the window is destroyed on drop.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("STATIC"),
                w!(""),
                WS_POPUP,
                0,
                0,
                width,
                height,
                None,
                None,
                None,
                None,
            )
        }
        .expect("create host window");
        Self(hwnd)
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        // SAFETY: owned by this thread.
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}

struct Case {
    text: ClockText,
    worst_case: WorstCase,
}
fn case(format: &str) -> Case {
    let prefs = Preferences {
        format: format.into(),
        ..Preferences::default()
    }
    .validate()
    .unwrap();
    let wall = FixedOffset::east_opt(0)
        .unwrap()
        .with_ymd_and_hms(2026, 9, 30, 14, 35, 0)
        .unwrap();
    Case {
        text: prefs.format().render(wall).unwrap(),
        worst_case: WorstCase::for_format(prefs.format()),
    }
}
fn request<'a>(case: &'a Case, dpi: u32, font_pt: u8) -> PaintRequest<'a> {
    PaintRequest {
        text: &case.text,
        worst_case: &case.worst_case,
        dpi,
        font_pt,
        palette: Palette::DARK,
    }
}
/// Typical Windows 11 clock-only region: 74 x 48 at 96 DPI (R03 hosted evidence).
fn region(dpi: u32) -> (i32, i32) {
    (
        clock_windows::render::scale_dip(74, dpi),
        clock_windows::render::scale_dip(48, dpi),
    )
}

#[test]
fn default_format_paints_at_every_supported_scale() {
    let renderer = Renderer::new().expect("renderer");
    let case = case(&Preferences::default().format);
    for dpi in [96, 120, 144, 192] {
        let (width, height) = region(dpi);
        let host = Host::new(width, height);
        let mut surface = renderer.create_surface(host.0);
        let outcome = surface.paint(&renderer, &request(&case, dpi, 9)).unwrap();
        assert_eq!(outcome, PaintOutcome::Painted, "{dpi} DPI");
    }
}
#[test]
fn identical_inputs_do_not_repaint_and_a_new_theme_does() {
    let renderer = Renderer::new().unwrap();
    let case = case("%H:%M");
    let (width, height) = region(96);
    let host = Host::new(width, height);
    let mut surface = renderer.create_surface(host.0);
    assert_eq!(
        surface.paint(&renderer, &request(&case, 96, 9)).unwrap(),
        PaintOutcome::Painted
    );
    assert_eq!(
        surface.paint(&renderer, &request(&case, 96, 9)).unwrap(),
        PaintOutcome::Unchanged
    );
    let light = PaintRequest {
        palette: Palette::LIGHT,
        ..request(&case, 96, 9)
    };
    assert_eq!(
        surface.paint(&renderer, &light).unwrap(),
        PaintOutcome::Painted
    );
    surface.mark_dirty();
    assert_eq!(
        surface.paint(&renderer, &light).unwrap(),
        PaintOutcome::Painted
    );
}
#[test]
fn long_format_reports_width_overflow_instead_of_truncating() {
    let renderer = Renderer::new().unwrap();
    let case = case("%A %d %B %Y");
    for dpi in [96, 192] {
        let (width, height) = region(dpi);
        let host = Host::new(width, height);
        let mut surface = renderer.create_surface(host.0);
        match surface.paint(&renderer, &request(&case, dpi, 9)).unwrap() {
            PaintOutcome::Overflow(overflow) => {
                assert!(overflow.width_exceeded(), "{dpi} DPI: {overflow:?}");
                assert_eq!(overflow.available, PixelSize { width, height });
            }
            other => panic!("{dpi} DPI: expected overflow, got {other:?}"),
        }
    }
}
#[test]
fn future_month_names_overflow_even_when_today_fits() {
    let renderer = Renderer::new().unwrap();
    // September 30 renders "September"; shrink the region until only the shorter
    // names fit, then require the verdict to come from the worst case.
    let prefs = Preferences {
        format: "%B".into(),
        ..Preferences::default()
    }
    .validate()
    .unwrap();
    let may = FixedOffset::east_opt(0)
        .unwrap()
        .with_ymd_and_hms(2026, 5, 4, 9, 0, 0)
        .unwrap();
    let may = Case {
        text: prefs.format().render(may).unwrap(),
        worst_case: WorstCase::for_format(prefs.format()),
    };
    let host = Host::new(70, 48);
    let mut surface = renderer.create_surface(host.0);
    let today_only = Case {
        text: may.text.clone(),
        worst_case: WorstCase::from_lines([may.text.lines.clone()]),
    };
    assert_eq!(
        surface
            .paint(&renderer, &request(&today_only, 96, 9))
            .unwrap(),
        PaintOutcome::Painted
    );
    assert!(matches!(
        surface.paint(&renderer, &request(&may, 96, 9)).unwrap(),
        PaintOutcome::Overflow(_)
    ));
}
#[test]
fn two_large_lines_overflow_the_height() {
    let renderer = Renderer::new().unwrap();
    let case = case("%H:%M\n%G-W%V");
    let (width, height) = region(96);
    let host = Host::new(width * 4, height);
    let mut surface = renderer.create_surface(host.0);
    match surface.paint(&renderer, &request(&case, 96, 24)).unwrap() {
        PaintOutcome::Overflow(overflow) => assert!(overflow.height_exceeded()),
        other => panic!("expected height overflow, got {other:?}"),
    }
}
#[test]
fn invalid_scale_is_an_error() {
    let renderer = Renderer::new().unwrap();
    let case = case("%H:%M");
    let host = Host::new(74, 48);
    let mut surface = renderer.create_surface(host.0);
    assert!(surface.paint(&renderer, &request(&case, 0, 9)).is_err());
}
#[test]
fn resizing_the_host_repaints_for_the_new_region() {
    use windows::Win32::UI::WindowsAndMessaging::{SWP_NOMOVE, SWP_NOZORDER, SetWindowPos};
    let renderer = Renderer::new().unwrap();
    let case = case("%H:%M");
    let host = Host::new(74, 48);
    let mut surface = renderer.create_surface(host.0);
    assert_eq!(
        surface.paint(&renderer, &request(&case, 96, 9)).unwrap(),
        PaintOutcome::Painted
    );
    // SAFETY: resizes our own window.
    unsafe { SetWindowPos(host.0, None, 0, 0, 92, 48, SWP_NOMOVE | SWP_NOZORDER) }.unwrap();
    assert_eq!(
        surface.paint(&renderer, &request(&case, 96, 9)).unwrap(),
        PaintOutcome::Painted
    );
}

fn counts() -> (u32, u32, u32) {
    let mut handles = 0;
    // SAFETY: pseudo-handle for the current process and a writable counter.
    unsafe {
        let process = GetCurrentProcess();
        GetProcessHandleCount(process, &mut handles).unwrap();
        (
            handles,
            GetGuiResources(process, GR_GDIOBJECTS),
            GetGuiResources(process, GR_USEROBJECTS),
        )
    }
}
fn cycle(renderer: &Renderer, surface: &mut ClockSurface, case: &Case) {
    surface.invalidate_device();
    assert_eq!(
        surface.paint(renderer, &request(case, 96, 9)).unwrap(),
        PaintOutcome::Painted
    );
}
#[test]
fn device_loss_recreation_and_surface_churn_do_not_leak_handles() {
    let renderer = Renderer::new().unwrap();
    let case = case("%H:%M\n%g%V.%u");
    let (width, height) = region(96);
    let host = Host::new(width, height);
    let mut surface = renderer.create_surface(host.0);
    // Warm one-time driver, font and shared-section allocations.
    for _ in 0..10 {
        cycle(&renderer, &mut surface, &case);
    }
    let before = counts();
    for _ in 0..200 {
        cycle(&renderer, &mut surface, &case);
    }
    for _ in 0..100 {
        let transient = renderer.create_surface(host.0);
        let mut transient = transient;
        assert_eq!(
            transient.paint(&renderer, &request(&case, 96, 9)).unwrap(),
            PaintOutcome::Painted
        );
    }
    let after = counts();
    assert!(
        after.0 <= before.0 + 8 && after.1 <= before.1 + 2 && after.2 <= before.2 + 2,
        "handles/GDI/USER grew: {before:?} -> {after:?}"
    );
}
#[test]
fn fit_is_exposed_for_layout_planning() {
    // The pure planner is public so R08 can size decisions without painting.
    let fit = Fit::Overflow(clock_windows::render::Overflow {
        required: PixelSize {
            width: 10,
            height: 10,
        },
        available: PixelSize {
            width: 5,
            height: 5,
        },
    });
    assert!(matches!(fit, Fit::Overflow(o) if o.width_exceeded() && o.height_exceeded()));
}
