//! Editable Settings drafts and immutable, validated runtime preferences.
use crate::{CompiledFormat, FormatError};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Monitors {
    Primary,
    All,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Theme {
    System,
    Light,
    Dark,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Preferences {
    pub format: String,
    pub monitors: Monitors,
    pub font_size_pt: u8,
    pub theme: Theme,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            format: "%H:%M\n%g%V.%u".into(),
            monitors: Monitors::Primary,
            font_size_pt: 9,
            theme: Theme::System,
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PreferencesError {
    FontSizeOutOfRange(i64),
    Format(FormatError),
    InvalidMonitors(String),
    InvalidTheme(String),
}
impl fmt::Display for PreferencesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FontSizeOutOfRange(_) => f.write_str("Font size must be between 6 and 24 pt."),
            Self::Format(e) => write!(f, "Invalid clock format: {e}"),
            Self::InvalidMonitors(_) => f.write_str("Unrecognized display selection."),
            Self::InvalidTheme(_) => f.write_str("Unrecognized theme."),
        }
    }
}
impl std::error::Error for PreferencesError {}
/// Fields are private so runtime consumers cannot bypass validation.
#[derive(Clone, Debug)]
pub struct ValidatedPreferences {
    draft: Preferences,
    compiled: CompiledFormat,
}
impl Preferences {
    pub fn validate(mut self) -> Result<ValidatedPreferences, PreferencesError> {
        if !(6..=24).contains(&self.font_size_pt) {
            return Err(PreferencesError::FontSizeOutOfRange(i64::from(
                self.font_size_pt,
            )));
        }
        let compiled = CompiledFormat::parse(&self.format).map_err(PreferencesError::Format)?;
        self.format = self.format.replace("\r\n", "\n");
        Ok(ValidatedPreferences {
            draft: self,
            compiled,
        })
    }
}
impl ValidatedPreferences {
    pub fn draft(&self) -> Preferences {
        self.draft.clone()
    }
    pub fn format(&self) -> &CompiledFormat {
        &self.compiled
    }
    pub fn format_source(&self) -> &str {
        &self.draft.format
    }
    pub fn monitors(&self) -> Monitors {
        self.draft.monitors
    }
    pub fn font_size_pt(&self) -> u8 {
        self.draft.font_size_pt
    }
    pub fn theme(&self) -> Theme {
        self.draft.theme
    }
}
impl TryFrom<&str> for Monitors {
    type Error = PreferencesError;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "primary" => Ok(Self::Primary),
            "all" => Ok(Self::All),
            _ => Err(PreferencesError::InvalidMonitors(value.into())),
        }
    }
}
impl TryFrom<&str> for Theme {
    type Error = PreferencesError;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "system" => Ok(Self::System),
            "light" => Ok(Self::Light),
            "dark" => Ok(Self::Dark),
            _ => Err(PreferencesError::InvalidTheme(value.into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_and_bounds() {
        let p = Preferences::default().validate().unwrap();
        assert_eq!(p.font_size_pt(), 9);
        assert_eq!(p.monitors(), Monitors::Primary);
        assert_eq!(p.theme(), Theme::System);
        for size in [0, 5, 25, 255] {
            assert_eq!(
                Preferences {
                    font_size_pt: size,
                    ..Preferences::default()
                }
                .validate()
                .unwrap_err(),
                PreferencesError::FontSizeOutOfRange(i64::from(size))
            );
        }
        for size in [6, 24] {
            assert!(
                Preferences {
                    font_size_pt: size,
                    ..Preferences::default()
                }
                .validate()
                .is_ok()
            );
        }
    }
    #[test]
    fn formats_and_enums() {
        assert!(matches!(
            Preferences {
                format: "%n".into(),
                ..Preferences::default()
            }
            .validate(),
            Err(PreferencesError::Format(_))
        ));
        let p = Preferences {
            format: "%H\r\n%M".into(),
            ..Preferences::default()
        }
        .validate()
        .unwrap();
        assert_eq!(p.format_source(), "%H\n%M");
        assert_eq!(
            Theme::try_from("unknown"),
            Err(PreferencesError::InvalidTheme("unknown".into()))
        );
        assert_eq!(
            Monitors::try_from("secondary"),
            Err(PreferencesError::InvalidMonitors("secondary".into()))
        );
    }
}
