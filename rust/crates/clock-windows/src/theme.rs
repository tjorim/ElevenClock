//! Clock palettes resolved from the preference and the platform appearance.
//!
//! The resolution rules are portable and unit tested. Only the system query at
//! the bottom touches Win32. No screen pixels are ever sampled: opaque colors come
//! from the documented per-user theme setting or the high-contrast system colors.
use clock_core::Theme;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}
impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
    /// WCAG relative luminance in 0.0..=1.0.
    fn luminance(self) -> f64 {
        let channel = |value: u8| {
            let c = f64::from(value) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(self.r) + 0.7152 * channel(self.g) + 0.0722 * channel(self.b)
    }
}
/// WCAG contrast ratio, 1.0 (identical) to 21.0 (black on white).
pub fn contrast_ratio(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (a.luminance(), b.luminance());
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Why a palette was chosen. High contrast always wins over the cosmetic setting.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PaletteKind {
    Light,
    Dark,
    HighContrast,
}
/// Fully opaque colors; there is no alpha channel by design.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Palette {
    pub kind: PaletteKind,
    pub background: Rgb,
    pub foreground: Rgb,
}
impl Palette {
    pub const LIGHT: Self = Self {
        kind: PaletteKind::Light,
        background: Rgb::new(243, 243, 243),
        foreground: Rgb::new(0, 0, 0),
    };
    pub const DARK: Self = Self {
        kind: PaletteKind::Dark,
        background: Rgb::new(32, 32, 32),
        foreground: Rgb::new(255, 255, 255),
    };
    /// Development harnesses that verify coverage by pixel color use fixed colors.
    pub const fn fixed(background: Rgb, foreground: Rgb) -> Self {
        Self {
            kind: PaletteKind::Dark,
            background,
            foreground,
        }
    }
}

/// The platform's resolved appearance, independent of the user's preference.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SystemAppearance {
    Light,
    Dark,
    HighContrast { background: Rgb, foreground: Rgb },
}

/// High contrast first, then an explicit Light/Dark choice, then the system.
pub fn resolve(preference: Theme, system: SystemAppearance) -> Palette {
    match (system, preference) {
        (
            SystemAppearance::HighContrast {
                background,
                foreground,
            },
            _,
        ) => Palette {
            kind: PaletteKind::HighContrast,
            background,
            foreground,
        },
        (_, Theme::Light) | (SystemAppearance::Light, Theme::System) => Palette::LIGHT,
        (_, Theme::Dark) | (SystemAppearance::Dark, Theme::System) => Palette::DARK,
    }
}

/// Turns repeated platform notifications into R06's resolved appearance revision.
/// Only a distinct resolved appearance produces a new revision, so unrelated
/// `WM_SETTINGCHANGE` traffic does not repaint every overlay.
#[derive(Clone, Debug)]
pub struct AppearanceTracker {
    current: SystemAppearance,
    revision: u64,
}
impl AppearanceTracker {
    pub fn new(initial: SystemAppearance) -> Self {
        Self {
            current: initial,
            revision: 0,
        }
    }
    pub fn current(&self) -> SystemAppearance {
        self.current
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// Returns the new revision for `Event::ThemeChanged` when the value changed.
    pub fn observe(&mut self, appearance: SystemAppearance) -> Option<u64> {
        if appearance == self.current {
            return None;
        }
        self.current = appearance;
        self.revision = self.revision.wrapping_add(1);
        Some(self.revision)
    }
}

#[cfg(windows)]
mod native {
    use super::{Rgb, SystemAppearance};
    use windows::Win32::{
        Foundation::LPARAM,
        Graphics::Gdi::{COLOR_WINDOW, COLOR_WINDOWTEXT, GetSysColor},
        System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW},
        UI::{
            Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW},
            WindowsAndMessaging::{
                SPI_GETHIGHCONTRAST, SystemParametersInfoW, WM_SETTINGCHANGE, WM_SYSCOLORCHANGE,
                WM_THEMECHANGED,
            },
        },
    };
    use windows::core::{PCWSTR, w};

    fn colorref(value: u32) -> Rgb {
        Rgb::new(value as u8, (value >> 8) as u8, (value >> 16) as u8)
    }
    fn high_contrast() -> bool {
        let mut info = HIGHCONTRASTW {
            cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32,
            ..Default::default()
        };
        // SAFETY: `info` is a correctly sized, writable HIGHCONTRASTW.
        unsafe {
            SystemParametersInfoW(
                SPI_GETHIGHCONTRAST,
                info.cbSize,
                Some((&mut info as *mut HIGHCONTRASTW).cast()),
                Default::default(),
            )
        }
        .is_ok()
            && info.dwFlags.contains(HCF_HIGHCONTRASTON)
    }
    /// `SystemUsesLightTheme` governs the taskbar surface. A missing value is the
    /// Windows default (light).
    fn system_uses_light_theme() -> bool {
        let mut data = 1u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        // SAFETY: the buffer and size describe `data`; the key/value are constants.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
                w!("SystemUsesLightTheme"),
                RRF_RT_REG_DWORD,
                None,
                Some((&mut data as *mut u32).cast()),
                Some(&mut size),
            )
        };
        status.is_err() || data != 0
    }

    /// Cheap enough for notification handlers; never call from a paint loop.
    pub fn system_appearance() -> SystemAppearance {
        if high_contrast() {
            // SAFETY: GetSysColor accepts these documented indices.
            let (background, foreground) =
                unsafe { (GetSysColor(COLOR_WINDOW), GetSysColor(COLOR_WINDOWTEXT)) };
            SystemAppearance::HighContrast {
                background: colorref(background),
                foreground: colorref(foreground),
            }
        } else if system_uses_light_theme() {
            SystemAppearance::Light
        } else {
            SystemAppearance::Dark
        }
    }

    /// True for messages after which the appearance must be re-read. The caller
    /// feeds the result to `AppearanceTracker::observe`.
    pub fn is_appearance_message(message: u32, lparam: LPARAM) -> bool {
        match message {
            WM_SYSCOLORCHANGE | WM_THEMECHANGED => true,
            WM_SETTINGCHANGE => {
                if lparam.0 == 0 {
                    return true;
                }
                // SAFETY: for WM_SETTINGCHANGE a nonzero LPARAM is a NUL-terminated
                // string valid for the duration of the message.
                let text =
                    unsafe { PCWSTR(lparam.0 as *const u16).to_string() }.unwrap_or_default();
                text == "ImmersiveColorSet" || text == "WindowsThemeElement"
            }
            _ => false,
        }
    }
}
#[cfg(windows)]
pub use native::{is_appearance_message, system_appearance};

#[cfg(test)]
mod tests {
    use super::*;

    const HC_BLACK: SystemAppearance = SystemAppearance::HighContrast {
        background: Rgb::new(0, 0, 0),
        foreground: Rgb::new(255, 255, 255),
    };

    #[test]
    fn explicit_theme_overrides_system_and_system_follows_it() {
        assert_eq!(
            resolve(Theme::Light, SystemAppearance::Dark),
            Palette::LIGHT
        );
        assert_eq!(resolve(Theme::Dark, SystemAppearance::Light), Palette::DARK);
        assert_eq!(
            resolve(Theme::System, SystemAppearance::Light),
            Palette::LIGHT
        );
        assert_eq!(
            resolve(Theme::System, SystemAppearance::Dark),
            Palette::DARK
        );
    }
    #[test]
    fn high_contrast_wins_over_every_cosmetic_choice() {
        for theme in [Theme::System, Theme::Light, Theme::Dark] {
            let palette = resolve(theme, HC_BLACK);
            assert_eq!(palette.kind, PaletteKind::HighContrast);
            assert_eq!(palette.background, Rgb::new(0, 0, 0));
            assert_eq!(palette.foreground, Rgb::new(255, 255, 255));
        }
    }
    #[test]
    fn built_in_palettes_are_readable() {
        assert!(contrast_ratio(Palette::LIGHT.background, Palette::LIGHT.foreground) >= 7.0);
        assert!(contrast_ratio(Palette::DARK.background, Palette::DARK.foreground) >= 7.0);
        assert!((contrast_ratio(Rgb::new(0, 0, 0), Rgb::new(255, 255, 255)) - 21.0).abs() < 1e-9);
        assert!((contrast_ratio(Rgb::new(9, 9, 9), Rgb::new(9, 9, 9)) - 1.0).abs() < 1e-9);
    }
    #[test]
    fn tracker_reports_only_distinct_appearance_changes() {
        let mut tracker = AppearanceTracker::new(SystemAppearance::Light);
        assert_eq!(tracker.observe(SystemAppearance::Light), None);
        assert_eq!(tracker.observe(SystemAppearance::Dark), Some(1));
        assert_eq!(tracker.observe(SystemAppearance::Dark), None);
        assert_eq!(tracker.observe(HC_BLACK), Some(2));
        // Different high-contrast colors are a different resolved appearance.
        let other = SystemAppearance::HighContrast {
            background: Rgb::new(255, 255, 255),
            foreground: Rgb::new(0, 0, 0),
        };
        assert_eq!(tracker.observe(other), Some(3));
        assert_eq!(tracker.current(), other);
        assert_eq!(tracker.revision(), 3);
    }
}
