use gpui::{Bounds, Hsla, Pixels, point, px, rgb, size};
use muxy_protocol::{Run, Size};

use super::{Palette, TerminalPane, push_quad, viewport_size};

#[derive(Clone, Copy)]
pub(super) struct Frame {
    outer: Bounds<Pixels>,
    pub(super) content: Bounds<Pixels>,
    pub(super) grid: Bounds<Pixels>,
    pub(super) viewport: Size,
    cell: gpui::Size<Pixels>,
    color: muxy_app_core::settings::PaddingColor,
}

impl Frame {
    #[cfg(test)]
    fn new(outer: Bounds<Pixels>, cell: gpui::Size<Pixels>) -> Self {
        Self::configured(
            outer,
            cell,
            &muxy_app_core::settings::TerminalOptions::default(),
        )
    }

    pub(super) fn configured(
        outer: Bounds<Pixels>,
        cell: gpui::Size<Pixels>,
        options: &muxy_app_core::settings::TerminalOptions,
    ) -> Self {
        let mut content = Bounds::new(
            outer.origin + point(px(options.padding_x[0]), px(options.padding_y[0])),
            size(
                (outer.size.width - px(options.padding_x.iter().sum())).max(px(0.0)),
                (outer.size.height - px(options.padding_y.iter().sum())).max(px(0.0)),
            ),
        );
        let viewport = viewport_size(content.size, cell);
        if options.padding_balance {
            let remainder = size(
                (content.size.width - cell.width * f32::from(viewport.cols)).max(px(0.0)),
                (content.size.height - cell.height * f32::from(viewport.rows)).max(px(0.0)),
            );
            content.origin += point(remainder.width / 2.0, remainder.height / 2.0);
            content.size = content.size - remainder;
        }
        Self {
            color: options.padding_color,
            outer,
            content,
            grid: Bounds::new(
                content.origin,
                size(
                    cell.width * f32::from(viewport.cols),
                    cell.height * f32::from(viewport.rows),
                ),
            ),
            viewport,
            cell,
        }
    }

    pub(super) fn backgrounds(
        self,
        view: &TerminalPane,
        palette: &Palette,
    ) -> Vec<(Bounds<Pixels>, Hsla)> {
        let mut quads = Vec::new();
        if self.color == muxy_app_core::settings::PaddingColor::Background {
            return quads;
        }
        let Some(grid) = view.displayed_grid() else {
            return quads;
        };
        let start = view.visible_start(grid);
        let remainder = view.scroll.pixel_remainder(f32::from(self.cell.height));
        for row in 0..=self.viewport.rows {
            if let Some(runs) = grid.content_row(start + usize::from(row)) {
                self.extend_row(runs, row, remainder, palette, &mut quads);
            }
        }
        quads
    }

    fn extend_row(
        self,
        runs: &[Run],
        row: u16,
        remainder: f32,
        palette: &Palette,
        quads: &mut Vec<(Bounds<Pixels>, Hsla)>,
    ) {
        let top = self.grid.top() + self.cell.height * f32::from(row) + px(remainder);
        let bottom = top + self.cell.height;
        if top >= self.grid.bottom() || bottom <= self.grid.top() {
            return;
        }
        let vertical = (top <= self.grid.top() || bottom >= self.grid.bottom())
            && (self.color == muxy_app_core::settings::PaddingColor::ExtendAlways
                || extend_vertical(runs, self.viewport.cols, palette));
        let mut column = 0_u16;
        for run in runs {
            let end = column.saturating_add(run.width).min(self.viewport.cols);
            if end <= column {
                continue;
            }
            let (foreground, background) = palette.style(run.style);
            let color = if run.text.starts_with('█') {
                foreground
            } else {
                background
            };
            if color != palette.background && (vertical || column == 0 || end == self.viewport.cols)
            {
                let left = if column == 0 {
                    self.outer.left()
                } else {
                    self.grid.left() + self.cell.width * f32::from(column)
                };
                let right = if end == self.viewport.cols {
                    self.outer.right()
                } else {
                    self.grid.left() + self.cell.width * f32::from(end)
                };
                let row_top = top.max(self.grid.top());
                let row_bottom = bottom.min(self.grid.bottom());
                for bounds in [
                    rect(left, row_top, self.grid.left().min(right), row_bottom),
                    rect(self.grid.right().max(left), row_top, right, row_bottom),
                    rect(
                        left,
                        self.outer.top(),
                        right,
                        if vertical && top <= self.grid.top() {
                            self.grid.top()
                        } else {
                            self.outer.top()
                        },
                    ),
                    rect(
                        left,
                        if vertical && bottom >= self.grid.bottom() {
                            self.grid.bottom()
                        } else {
                            self.outer.bottom()
                        },
                        right,
                        self.outer.bottom(),
                    ),
                ] {
                    let bounds = bounds.intersect(&self.outer);
                    if bounds.size.width > px(0.0) && bounds.size.height > px(0.0) {
                        push_quad(quads, bounds, rgb(color).into());
                    }
                }
            }
            column = end;
            if column == self.viewport.cols {
                break;
            }
        }
    }
}

fn rect(left: Pixels, top: Pixels, right: Pixels, bottom: Pixels) -> Bounds<Pixels> {
    Bounds::new(point(left, top), size(right - left, bottom - top))
}

fn extend_vertical(runs: &[Run], cols: u16, palette: &Palette) -> bool {
    let mut width = 0_u16;
    for run in runs {
        if run.width == 0 {
            continue;
        }
        if palette.resolve(run.style.bg, palette.background) == palette.background
            || run.text.chars().any(|character| {
                matches!(character, '\u{e0b0}'..='\u{e0c8}' | '\u{e0ca}' | '\u{e0cc}'..='\u{e0d2}' | '\u{e0d4}')
            })
        {
            return false;
        }
        width = width.saturating_add(run.width);
        if width >= cols {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use muxy_protocol::{Color, Style};

    fn run(text: &str, width: u16, bg: Color) -> Run {
        Run {
            text: text.into(),
            width,
            style: Style {
                bg,
                ..Style::default()
            },
        }
    }

    fn frame() -> Frame {
        Frame {
            color: muxy_app_core::settings::PaddingColor::default(),
            outer: rect(px(10.0), px(20.0), px(41.0), px(63.0)),
            content: rect(px(12.0), px(22.0), px(39.0), px(61.0)),
            grid: rect(px(12.0), px(22.0), px(36.0), px(54.0)),
            viewport: Size { cols: 3, rows: 2 },
            cell: size(px(8.0), px(16.0)),
        }
    }

    fn color_at(quads: &[(Bounds<Pixels>, Hsla)], x: f32, y: f32) -> Option<Hsla> {
        let mut hits = quads.iter().filter(|(bounds, _)| {
            bounds.left() <= px(x)
                && bounds.right() > px(x)
                && bounds.top() <= px(y)
                && bounds.bottom() > px(y)
        });
        let color = hits.next().map(|(_, color)| *color);
        assert!(hits.next().is_none(), "padding quads must not overlap");
        color
    }

    #[test]
    fn configured_padding_preserves_asymmetry_and_balances_unused_cells() {
        let outer = Bounds::new(point(px(0.0), px(0.0)), size(px(100.0), px(80.0)));
        let cell = size(px(8.0), px(16.0));
        let mut options = muxy_app_core::settings::TerminalOptions {
            padding_x: [4.0, 8.0],
            padding_y: [3.0, 5.0],
            ..Default::default()
        };
        let frame = Frame::configured(outer, cell, &options);
        assert_eq!(frame.grid.origin, point(px(4.0), px(3.0)));
        assert_eq!(frame.viewport, Size { cols: 11, rows: 4 });
        options.padding_balance = true;
        let frame = Frame::configured(outer, cell, &options);
        assert_eq!(frame.grid.origin, point(px(4.0), px(7.0)));
        assert_eq!(frame.content.size, frame.grid.size);
    }

    #[test]
    fn frame_keeps_fractional_cell_space_on_right_and_bottom() {
        let frame = Frame::new(frame().outer, frame().cell);
        assert_eq!(frame.grid.origin, frame.content.origin);
        assert_eq!(
            frame.grid.size.width,
            frame.cell.width * f32::from(frame.viewport.cols)
        );
        assert_eq!(
            frame.grid.size.height,
            frame.cell.height * f32::from(frame.viewport.rows)
        );
        let remainder = frame.content.size - frame.grid.size;
        assert!(remainder.width >= px(0.0) && remainder.width < frame.cell.width);
        assert!(remainder.height >= px(0.0) && remainder.height < frame.cell.height);
        let tiny = Frame::new(Bounds::default(), size(px(8.0), px(16.0)));
        assert_eq!(tiny.content.size, size(px(0.0), px(0.0)));
        assert_eq!(tiny.viewport, Size { cols: 1, rows: 1 });
    }

    #[test]
    fn colored_rows_extend_to_all_edges_and_corners_without_covering_cells() {
        let frame = frame();
        let mut quads = Vec::new();
        let red = Color::Rgb(255, 0, 0);
        let blue = Color::Rgb(0, 0, 255);
        frame.extend_row(
            &[run("x", 1, red), run("  ", 2, blue)],
            0,
            0.0,
            &Palette::new(true),
            &mut quads,
        );
        frame.extend_row(
            &[run("   ", 3, blue)],
            1,
            0.0,
            &Palette::new(true),
            &mut quads,
        );
        for (x, y, color) in [
            (10.5, 20.5, 0xff_00_00),
            (10.5, 30.0, 0xff_00_00),
            (20.5, 20.5, 0x00_00_ff),
            (40.5, 20.5, 0x00_00_ff),
            (40.5, 30.0, 0x00_00_ff),
            (10.5, 62.5, 0x00_00_ff),
            (20.5, 62.5, 0x00_00_ff),
            (40.5, 62.5, 0x00_00_ff),
        ] {
            assert_eq!(color_at(&quads, x, y), Some(rgb(color).into()));
        }
        for (bounds, _) in quads {
            assert!(!bounds.intersects(&frame.grid));
            assert_eq!(bounds.intersect(&frame.outer), bounds);
        }
    }

    #[test]
    fn default_backgrounds_missing_cells_and_powerline_disable_vertical_extension() {
        let palette = Palette::new(true);
        let [red, green, blue] = palette.terminal_colors().background;
        let background = Color::Rgb(red, green, blue);
        let red = Color::Rgb(255, 0, 0);
        for runs in [
            vec![run("x", 1, red)],
            vec![run("x", 1, red), run("  ", 2, Color::Default)],
            vec![run("x", 1, red), run("  ", 2, background)],
            vec![run("\u{e0b0}", 1, red), run("  ", 2, red)],
            vec![run("\u{e0d4}", 1, red), run("  ", 2, red)],
        ] {
            assert!(!extend_vertical(&runs, 3, &palette));
            let mut quads = Vec::new();
            frame().extend_row(&runs, 0, 0.0, &palette, &mut quads);
            assert_eq!(color_at(&quads, 10.5, 30.0), Some(rgb(0xff_00_00).into()));
            assert_eq!(color_at(&quads, 10.5, 20.5), None);
            assert_eq!(color_at(&quads, 20.5, 20.5), None);
        }
        assert!(extend_vertical(&[run("   ", 3, red)], 3, &palette));
    }

    #[test]
    fn inverse_and_covering_blocks_use_the_visible_edge_color() {
        let palette = Palette::new(true);
        for (text, inverse, expected) in [
            ("x", false, 0x00_00_ff),
            ("x", true, 0xff_00_00),
            ("█", false, 0xff_00_00),
            ("█", true, 0x00_00_ff),
        ] {
            let run = Run {
                text: text.into(),
                width: 3,
                style: Style {
                    fg: Color::Rgb(255, 0, 0),
                    bg: Color::Rgb(0, 0, 255),
                    inverse,
                    ..Style::default()
                },
            };
            let mut quads = Vec::new();
            frame().extend_row(&[run], 0, 0.0, &palette, &mut quads);
            for (x, y) in [(10.5, 30.0), (40.5, 30.0), (20.5, 20.5)] {
                assert_eq!(color_at(&quads, x, y), Some(rgb(expected).into()));
            }
        }
    }

    #[test]
    fn fractional_scroll_extends_the_actual_visible_edge_rows() {
        let frame = frame();
        let palette = Palette::new(true);
        let mut quads = Vec::new();
        for (row, color) in [
            (0, Color::Rgb(255, 0, 0)),
            (1, Color::Rgb(0, 255, 0)),
            (2, Color::Rgb(0, 0, 255)),
        ] {
            frame.extend_row(&[run("   ", 3, color)], row, -4.0, &palette, &mut quads);
        }
        for (y, color) in [
            (20.5, 0xff_00_00),
            (33.5, 0xff_00_00),
            (34.5, 0x00_ff_00),
            (50.5, 0x00_00_ff),
            (62.5, 0x00_00_ff),
        ] {
            assert_eq!(color_at(&quads, 40.5, y), Some(rgb(color).into()));
        }
    }
}
