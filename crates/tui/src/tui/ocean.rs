//! Terminal-native underwater field for the CodeWhale transcript.
//!
//! The field is atmosphere, never content: callers paint it only into cells
//! outside occupied transcript text. Reduced motion freezes the field but does
//! not remove it, so choosing an underwater treatment always has a visible
//! result.

use ratatui::style::Color;

use crate::palette::{PaletteMode, UiTheme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OceanRamp {
    pub surface: Color,
    pub middle: Color,
    pub deep: Color,
    pub ambient: Color,
}

impl OceanRamp {
    #[must_use]
    pub fn for_theme(theme: &UiTheme) -> Option<Self> {
        let base = rgb(theme.surface_bg)?;
        let seafoam = rgb(theme.accent_secondary).unwrap_or((79, 209, 197));

        let (surface, middle, deep) = match theme.mode {
            PaletteMode::Light | PaletteMode::SolarizedLight => (
                mix(base, seafoam, 0.07),
                mix(base, seafoam, 0.13),
                mix(base, (70, 139, 196), 0.18),
            ),
            PaletteMode::Dark | PaletteMode::Grayscale => (
                mix(base, (30, 71, 103), 0.24),
                mix(base, (7, 30, 54), 0.40),
                mix(base, (2, 9, 24), 0.64),
            ),
        };

        Some(Self {
            surface: color(surface),
            middle: color(middle),
            deep: color(deep),
            ambient: color(mix(seafoam, base, 0.42)),
        })
    }

    #[must_use]
    pub fn color_at(self, row: u16, height: u16) -> Color {
        if height <= 1 {
            return self.surface;
        }
        let position = f32::from(row.min(height - 1)) / f32::from(height - 1);
        if position <= 0.42 {
            mix_colors(self.surface, self.middle, position / 0.42)
        } else {
            mix_colors(self.middle, self.deep, (position - 0.42) / 0.58)
        }
    }

    #[must_use]
    pub fn color_at_phase(self, row: u16, height: u16, elapsed_ms: u128) -> Color {
        let base = self.color_at(row, height);
        let depth = if height <= 1 {
            0.0
        } else {
            f32::from(row.min(height - 1)) / f32::from(height - 1)
        };
        let cycle = (elapsed_ms % 18_000) as f32 / 18_000.0;
        let breath = (cycle * std::f32::consts::TAU).sin() * 0.5 + 0.5;
        mix_colors(base, self.ambient, breath * 0.045 * (1.0 - depth))
    }
}

#[must_use]
fn rgb(value: Color) -> Option<(u8, u8, u8)> {
    match value {
        Color::Rgb(r, g, b) => Some((r, g, b)),
        _ => None,
    }
}

#[must_use]
fn color((r, g, b): (u8, u8, u8)) -> Color {
    Color::Rgb(r, g, b)
}

#[must_use]
fn mix_colors(from: Color, to: Color, amount: f32) -> Color {
    match (rgb(from), rgb(to)) {
        (Some(from), Some(to)) => color(mix(from, to, amount)),
        _ => from,
    }
}

#[must_use]
fn mix(from: (u8, u8, u8), to: (u8, u8, u8), amount: f32) -> (u8, u8, u8) {
    let amount = amount.clamp(0.0, 1.0);
    let channel = |a: u8, b: u8| {
        (f32::from(a) + (f32::from(b) - f32::from(a)) * amount)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    (
        channel(from.0, to.0),
        channel(from.1, to.1),
        channel(from.2, to.2),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn distance(a: Color, b: Color) -> u16 {
        let (ar, ag, ab) = rgb(a).expect("RGB color");
        let (br, bg, bb) = rgb(b).expect("RGB color");
        ar.abs_diff(br) as u16 + ag.abs_diff(bg) as u16 + ab.abs_diff(bb) as u16
    }

    #[test]
    fn whale_ramp_is_perceptibly_deep_not_merely_non_equal() {
        let ramp = OceanRamp::for_theme(&crate::palette::UI_THEME).expect("RGB theme");
        assert!(
            distance(ramp.surface, ramp.deep) >= 32,
            "the selected underwater treatment must read at a glance"
        );
        assert_ne!(ramp.color_at(0, 20), ramp.color_at(19, 20));
    }

    #[test]
    fn light_theme_stays_light_enough_for_light_theme_text() {
        let ramp = OceanRamp::for_theme(&crate::palette::LIGHT_UI_THEME).expect("RGB theme");
        let (r, g, b) = rgb(ramp.deep).expect("RGB color");
        assert!(u16::from(r) + u16::from(g) + u16::from(b) > 420);
    }

    #[test]
    fn inherited_terminal_background_reports_no_ramp() {
        let mut theme = crate::palette::UI_THEME;
        theme.surface_bg = Color::Reset;
        assert_eq!(OceanRamp::for_theme(&theme), None);
    }

    #[test]
    fn shimmer_is_subtle_and_concentrated_near_the_surface() {
        let ramp = OceanRamp::for_theme(&crate::palette::UI_THEME).expect("RGB theme");
        let surface_a = ramp.color_at_phase(0, 20, 0);
        let surface_b = ramp.color_at_phase(0, 20, 4_500);
        let deep_a = ramp.color_at_phase(19, 20, 0);
        let deep_b = ramp.color_at_phase(19, 20, 4_500);

        let surface_shift = distance(surface_a, surface_b);
        assert!(
            (1..=12).contains(&surface_shift),
            "surface shift was {surface_shift}"
        );
        assert_eq!(
            deep_a, deep_b,
            "the floor should stay perceptually anchored"
        );
    }
}
