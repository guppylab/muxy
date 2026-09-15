use std::collections::BTreeSet;

use gpui::{Bounds, Pixels, point, px, size};

pub(super) fn uncovered(
    outer: Bounds<Pixels>,
    colored: impl Iterator<Item = Bounds<Pixels>>,
) -> Vec<Bounds<Pixels>> {
    let rectangles: Vec<_> = colored
        .map(|bounds| bounds.intersect(&outer))
        .filter(|bounds| bounds.size.width > px(0.0) && bounds.size.height > px(0.0))
        .collect();
    let mut events = vec![
        (outer.top(), usize::MAX, false),
        (outer.bottom(), usize::MAX, false),
    ];
    for (index, bounds) in rectangles.iter().enumerate() {
        events.push((bounds.top(), index, true));
        events.push((bounds.bottom(), index, false));
    }
    events.sort_by(|a, b| f32::from(a.0).total_cmp(&f32::from(b.0)));
    let mut result = Vec::new();
    let mut active = BTreeSet::new();
    let mut top = outer.top();
    for (bottom, index, starts) in events {
        if bottom > top {
            let mut intervals: Vec<&Bounds<Pixels>> =
                active.iter().map(|&index| &rectangles[index]).collect();
            intervals.sort_by(|a, b| f32::from(a.left()).total_cmp(&f32::from(b.left())));
            let mut left = outer.left();
            for bounds in intervals {
                if bounds.left() > left {
                    result.push(Bounds::new(
                        point(left, top),
                        size(bounds.left() - left, bottom - top),
                    ));
                }
                left = left.max(bounds.right());
            }
            if left < outer.right() {
                result.push(Bounds::new(
                    point(left, top),
                    size(outer.right() - left, bottom - top),
                ));
            }
        }
        if index != usize::MAX {
            if starts {
                active.insert(index);
            } else {
                active.remove(&index);
            }
        }
        top = bottom;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translucent_background_tiles_cover_each_pixel_once() {
        let bounds = |x, y, w, h| Bounds::new(point(px(x), px(y)), size(px(w), px(h)));
        let outer = bounds(0.0, 0.0, 8.0, 6.0);
        let colored = [
            bounds(0.0, 1.0, 2.0, 4.0),
            bounds(2.0, 2.0, 5.0, 2.0),
            bounds(7.0, 1.0, 1.0, 5.0),
        ];
        let defaults = uncovered(outer, colored.into_iter());
        for y in 0_u16..6 {
            for x in 0_u16..8 {
                let point = point(px(f32::from(x) + 0.5), px(f32::from(y) + 0.5));
                let mut opacity = 0.0_f32;
                for tile in defaults.iter().chain(&colored) {
                    if tile.contains(&point) {
                        opacity = 0.5 + opacity * 0.5;
                    }
                }
                assert_eq!(opacity.to_bits(), 0.5_f32.to_bits(), "pixel {x},{y}");
            }
        }
    }
}
