use gpui::{Rgba, rgb};
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct ColorScheme {
    pub background: Option<Rgba>,
    pub foreground: Option<Rgba>,
    pub cursor_color: Option<CellColor>,
    pub cursor_text: Option<CellColor>,
    pub selection_foreground: Option<CellColor>,
    pub selection_background: Option<CellColor>,
    pub palette: HashMap<usize, Rgba>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CellColor {
    Rgb(Rgba),
    Foreground,
    Background,
}

impl CellColor {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "cell-foreground" => Some(Self::Foreground),
            "cell-background" => Some(Self::Background),
            _ => parse_hex(value).map(Self::Rgb),
        }
    }
}

impl ColorScheme {
    pub fn parse(source: &str) -> Self {
        let mut theme = Self::default();
        for line in source.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();
            match key {
                "background" => theme.background = parse_hex(value),
                "foreground" => theme.foreground = parse_hex(value),
                "cursor-color" => theme.cursor_color = CellColor::parse(value),
                "cursor-text" => theme.cursor_text = CellColor::parse(value),
                "selection-foreground" => theme.selection_foreground = CellColor::parse(value),
                "selection-background" => theme.selection_background = CellColor::parse(value),
                "palette" => {
                    let Some((index, color)) = value.split_once('=') else {
                        continue;
                    };
                    let (Ok(index), Some(color)) = (index.trim().parse(), parse_hex(color)) else {
                        continue;
                    };
                    theme.palette.insert(index, color);
                }
                _ => {}
            }
        }
        theme
    }

    pub fn palette_color(&self, index: usize) -> Option<Rgba> {
        self.palette.get(&index).copied()
    }
}

pub fn parse_hex(value: &str) -> Option<Rgba> {
    let value = value.trim().trim_start_matches('#');
    if value.len() != 6 {
        return None;
    }
    u32::from_str_radix(value, 16).ok().map(rgb)
}

pub(super) fn luminance(color: Rgba) -> f32 {
    0.2126 * color.r + 0.7152 * color.g + 0.0722 * color.b
}

pub(super) fn with_alpha(color: Rgba, alpha: f32) -> Rgba {
    Rgba { a: alpha, ..color }
}
