use gpui::{Bounds, Hsla, Pixels, point, px, rgb, size};
use muxy_protocol::{Style, Underline};

use crate::views::terminal::colors::Palette;

pub(super) fn prepare(
    quads: &mut Vec<(Bounds<Pixels>, Hsla)>,
    style: Style,
    bounds: Bounds<Pixels>,
    foreground: Hsla,
    palette: &Palette,
    scale: f32,
) {
    let snap = |value: Pixels| px((f32::from(value) * scale).round() / scale);
    let thickness = px(1.0 / scale);
    let left = snap(bounds.left());
    let right = snap(bounds.right());
    let bottom = snap(bounds.bottom()) - thickness * 2.0;
    let mut line = |y, color| {
        quads.push((
            Bounds::new(point(left, y), size(right - left, thickness)),
            color,
        ));
    };
    if style.strikethrough {
        line(snap(bounds.top() + bounds.size.height / 2.0), foreground);
    }
    if style.overline {
        line(snap(bounds.top()), foreground);
    }
    let mut color: Hsla =
        rgb(palette.resolve(style.underline_color, palette.style(style).0)).into();
    color.a = foreground.a;
    match style.underline {
        Underline::None => {}
        Underline::Single => line(bottom, color),
        Underline::Double => {
            line(bottom, color);
            line(bottom - thickness * 2.0, color);
        }
        Underline::Dotted | Underline::Dashed | Underline::Curly => {
            let width = if style.underline == Underline::Dashed {
                3.0
            } else {
                1.0
            };
            let step = if style.underline == Underline::Curly {
                1.0
            } else {
                width + 1.0
            };
            let mut x = left;
            while x < right {
                let phase = (f32::from(x) * scale).rem_euclid(4.0);
                let y = if style.underline == Underline::Curly {
                    bottom - thickness * (phase - 2.0).abs()
                } else {
                    bottom
                };
                quads.push((
                    Bounds::new(
                        point(x, y),
                        size((thickness * width).min(right - x), thickness),
                    ),
                    color,
                ));
                x += thickness * step;
            }
        }
    }
}
