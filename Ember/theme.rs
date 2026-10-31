use ember_core::{Preferences, ThemeMode};
use ferroui_base::controls::ResourceKey;
use ferroui_base::media::{Color, SolidColorBrush};
use ferroui_base::styling::ThemeVariant;
use ferroui_controls::Application;
use ferroui_themes_winui::accents::SystemAccentColors;
use ferroui_themes_winui::{DensityStyle, WinUITheme};
use std::rc::Rc;
pub(crate) fn set_accent(theme: &WinUITheme, hex: &str) {
    let Ok(accent) = Color::parse(hex) else {
        return;
    };
    let (d1, d2, d3, l1, l2, l3) = SystemAccentColors::calculate_accent_shades(accent);
    // Small accent labels and selection indicators share one ink per theme.
    // Use the lowest-contrast reading surface in each built-in palette.
    for (keys, surface, ink) in [
        (
            [
                "EmberLightAccentColor",
                "EmberLightAccentHover",
                "EmberLightAccentPressed",
            ],
            Color::from_rgb(232, 227, 222),
            Color::from_rgb(0, 0, 0),
        ),
        (
            [
                "EmberDarkAccentColor",
                "EmberDarkAccentHover",
                "EmberDarkAccentPressed",
            ],
            Color::from_rgb(63, 54, 49),
            Color::from_rgb(255, 255, 255),
        ),
    ] {
        for (key, color) in keys.into_iter().zip(accent_states(accent, surface, ink)) {
            theme.resources().set(key, Some(Rc::new(color)));
        }
    }
    let keys = [
        SystemAccentColors::ACCENT_KEY,
        SystemAccentColors::ACCENT_DARK1_KEY,
        SystemAccentColors::ACCENT_DARK2_KEY,
        SystemAccentColors::ACCENT_DARK3_KEY,
        SystemAccentColors::ACCENT_LIGHT1_KEY,
        SystemAccentColors::ACCENT_LIGHT2_KEY,
        SystemAccentColors::ACCENT_LIGHT3_KEY,
    ];
    for (key, color) in keys.into_iter().zip([accent, d1, d2, d3, l1, l2, l3]) {
        theme.resources().set(key, Some(Rc::new(color)));
    }
}
pub(crate) fn apply(preferences: &Preferences) {
    if let Some(application) = Application::current() {
        // Overrides live in the ordinary dictionary; the built-in Light/Dark
        // dictionaries remain intact and become visible again on a mode change.
        for key in CUSTOM_KEYS {
            application.resources().remove(&ResourceKey::from(key));
        }
        let custom = (preferences.theme == ThemeMode::Custom).then(|| {
            let background = preferences
                .custom_background
                .as_deref()
                .and_then(|s| Color::parse(s).ok())
                .unwrap_or(Color::from_rgb(24, 27, 31));
            let accent = Color::parse(&preferences.accent).unwrap_or(Color::from_rgb(255, 107, 53));
            custom_palette(background, accent)
        });
        application.set_requested_theme_variant(match preferences.theme {
            ThemeMode::System => None,
            ThemeMode::Light => Some(ThemeVariant::light()),
            ThemeMode::Dark => Some(ThemeVariant::dark()),
            ThemeMode::Custom => Some(if custom.as_ref().is_some_and(|p| p.dark) {
                ThemeVariant::dark()
            } else {
                ThemeVariant::light()
            }),
        });
        if let Some(palette) = custom {
            for (key, color) in CUSTOM_KEYS.into_iter().zip(palette.colors) {
                application
                    .resources()
                    .set(key, Some(Rc::new(SolidColorBrush::with_color(color))));
            }
        }
        // Available to markup and future reading controls without scaling the
        // navigation chrome. Rich HN content already consumes these preferences.
        application
            .resources()
            .set("EmberReadingFontSize", Some(Rc::new(preferences.font_size)));
        application.resources().set(
            "EmberReadingLineHeight",
            Some(Rc::new(preferences.font_size * preferences.line_height)),
        );
    }
    if let Some(theme) = crate::App::theme() {
        set_accent(&theme, &preferences.accent);
        theme.set_density_style(if preferences.compact {
            DensityStyle::Compact
        } else {
            DensityStyle::Normal
        });
    }
}

const CUSTOM_KEYS: [&str; 10] = [
    "EmberBackground",
    "EmberRail",
    "EmberSurface",
    "EmberCard",
    "EmberBorder",
    "EmberText",
    "EmberMuted",
    "EmberTint",
    "EmberCode",
    "EmberAccentInk",
];
struct CustomPalette {
    dark: bool,
    colors: [Color; 10],
}

/// The base variant follows relative sRGB luminance. Derived reading surfaces
/// keep at least 4.5:1 contrast for both body and secondary text, including
/// difficult mid-tone custom colors where an arbitrary fixed gray would fail.
fn custom_palette(background: Color, accent: Color) -> CustomPalette {
    let white = Color::from_rgb(255, 255, 255);
    let black = Color::from_rgb(0, 0, 0);
    let dark = luminance(background) < 0.179;
    let text = if dark { white } else { black };
    let surface = safe_surface(background, white, if dark { 0.07 } else { 0.38 }, text);
    let card = safe_surface(background, white, if dark { 0.11 } else { 0.60 }, text);
    let rail = safe_surface(background, if dark { black } else { white }, 0.12, text);
    let code = safe_surface(background, if dark { black } else { white }, 0.22, text);
    let tint = safe_surface(surface, accent, 0.14, text);
    let border = blend(background, text, 0.22);
    let surfaces = [background, rail, surface, card, tint, code];
    let muted = readable_ink(blend(text, background, 0.35), text, &surfaces);
    let accent_ink = readable_ink(accent, text, &surfaces);
    CustomPalette {
        dark,
        colors: [
            background, rail, surface, card, border, text, muted, tint, code, accent_ink,
        ],
    }
}

// The resting fill doubles as accent text on the reading surfaces. Hover and
// press use opaque shades so their label contrast holds over any custom canvas.
fn accent_states(accent: Color, surface: Color, ink: Color) -> [Color; 3] {
    let rest = readable_ink(accent, ink, &[surface]);
    // Make states distinct even for an accent at either end of the tonal range.
    let toward = if contrast(rest, surface) >= 9.0 {
        surface
    } else {
        ink
    };
    [
        rest,
        safe_surface(rest, toward, 0.10, surface),
        safe_surface(rest, toward, 0.20, surface),
    ]
}

fn readable_ink(preferred: Color, fallback: Color, surfaces: &[Color]) -> Color {
    for step in 0..=32 {
        let candidate = blend(preferred, fallback, step as f64 / 32.0);
        if surfaces
            .iter()
            .all(|&surface| contrast(candidate, surface) >= 4.5)
        {
            return candidate;
        }
    }
    fallback
}

fn safe_surface(base: Color, target: Color, amount: f64, foreground: Color) -> Color {
    for step in (0..=20).rev() {
        let candidate = blend(base, target, amount * step as f64 / 20.0);
        if contrast(candidate, foreground) >= 4.5 {
            return candidate;
        }
    }
    base
}
fn blend(a: Color, b: Color, amount: f64) -> Color {
    let channel = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * amount).round() as u8;
    Color::from_rgb(channel(a.r, b.r), channel(a.g, b.g), channel(a.b, b.b))
}
fn luminance(color: Color) -> f64 {
    let linear = |value: u8| {
        let value = value as f64 / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
}
fn contrast(a: Color, b: Color) -> f64 {
    let a = luminance(a);
    let b = luminance(b);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn primary_actions_keep_contrast_and_distinct_pointer_states_for_any_accent() {
        let white = Color::from_rgb(255, 255, 255);
        let black = Color::from_rgb(0, 0, 0);
        for (surface, ink, label) in [
            (Color::from_rgb(232, 227, 222), black, white),
            (Color::from_rgb(63, 54, 49), white, black),
        ] {
            for r in (0..=255).step_by(17) {
                for g in (0..=255).step_by(17) {
                    for b in (0..=255).step_by(17) {
                        let [normal, hover, pressed] =
                            accent_states(Color::from_rgb(r, g, b), surface, ink);
                        assert_ne!(normal, hover);
                        assert_ne!(hover, pressed);
                        for fill in [normal, hover, pressed] {
                            assert!(contrast(label, fill) >= 4.5);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn custom_palettes_keep_readable_body_and_secondary_text() {
        for hex in [
            "#000000", "#FFFFFF", "#747474", "#777777", "#172126", "#FFEFC0", "#0055FF", "#CA2070",
        ] {
            let palette = custom_palette(Color::parse(hex).unwrap(), Color::from_rgb(255, 107, 53));
            for index in [0, 1, 2, 3, 7, 8] {
                assert!(
                    contrast(palette.colors[9], palette.colors[index]) >= 4.5,
                    "{hex}, accent ink on surface {index}"
                );
                assert!(
                    contrast(palette.colors[5], palette.colors[index]) >= 4.5,
                    "{hex}, surface {index}"
                );
                assert!(
                    contrast(palette.colors[6], palette.colors[index]) >= 4.5,
                    "{hex}, muted on {index}"
                );
            }
        }
    }
    #[test]
    fn custom_theme_variant_tracks_background_luminance() {
        assert!(custom_palette(Color::from_rgb(24, 27, 31), Color::from_rgb(255, 100, 20)).dark);
        assert!(
            !custom_palette(
                Color::from_rgb(250, 240, 220),
                Color::from_rgb(255, 100, 20)
            )
            .dark
        );
    }
}
