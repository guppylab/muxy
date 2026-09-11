use std::{cell::Cell, cell::RefCell, rc::Rc};

use gpui::{
    AnyElement, Bounds, DispatchPhase, Hsla, InteractiveElement, IntoElement, ListOffset,
    ListState, MouseButton, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Size, Styled,
    canvas, div, fill, point, px, size,
};

use super::{MINIMUM_THUMB_LENGTH, ThumbGeometry};

#[derive(Clone, Debug, Default)]
pub struct ListScrollbar {
    state: Rc<RefCell<ScrollbarState>>,
}

#[derive(Debug, Default)]
struct ScrollbarState {
    heights: Vec<Option<Pixels>>,
    viewport: Size<Pixels>,
    drag: Option<Drag>,
}

#[derive(Debug)]
struct Drag {
    extent: Extent,
    grab: Pixels,
    offset: Pixels,
}

#[derive(Debug)]
struct Extent {
    heights: Vec<Pixels>,
    total: Pixels,
    visible: Pixels,
}

impl ListScrollbar {
    pub fn reset(&self) {
        *self.state.borrow_mut() = ScrollbarState::default();
    }

    pub fn element(&self, list: &ListState, color: Hsla) -> AnyElement {
        let list = list.clone();
        let press_list = list.clone();
        let state = self.state.clone();
        let press_state = state.clone();
        let track = Rc::new(Cell::new(None));
        let press_track = track.clone();
        div()
            .w(px(10.0))
            .h_full()
            .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                let Some(bounds) = press_track.get() else {
                    return;
                };
                let mut state = press_state.borrow_mut();
                state.refresh(&press_list);
                let extent = state.extent();
                let offset = extent.offset(&press_list);
                let Some(thumb) = extent.thumb_bounds(bounds, offset) else {
                    return;
                };
                let grab = if thumb.contains(&event.position) {
                    event.position.y - thumb.top()
                } else {
                    thumb.size.height / 2.0
                };
                let mut drag = Drag {
                    extent,
                    grab,
                    offset,
                };
                drag.scroll_to_pointer(&press_list, bounds, event.position.y);
                state.drag = Some(drag);
                window.refresh();
                cx.stop_propagation();
            })
            .child(
                canvas(
                    move |bounds, _, _| {
                        track.set(Some(bounds));
                        bounds
                    },
                    move |_, bounds, window, _| {
                        let mut scrollbar = state.borrow_mut();
                        scrollbar.refresh(&list);
                        if let Some(thumb) = scrollbar.thumb_bounds(&list, bounds) {
                            window.paint_quad(fill(thumb, color).corner_radii(px(3.0)));
                        }
                        let move_list = list.clone();
                        let move_state = state.clone();
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                            if phase == DispatchPhase::Bubble
                                && event.pressed_button == Some(MouseButton::Left)
                                && let Some(drag) = &mut move_state.borrow_mut().drag
                            {
                                drag.scroll_to_pointer(&move_list, bounds, event.position.y);
                                window.refresh();
                                cx.stop_propagation();
                            }
                        });
                        let end_state = state.clone();
                        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, _| {
                            if phase == DispatchPhase::Capture
                                && event.button == MouseButton::Left
                                && end_state.borrow_mut().drag.take().is_some()
                            {
                                window.refresh();
                            }
                        });
                    },
                )
                .size_full(),
            )
            .into_any_element()
    }
}

impl ScrollbarState {
    fn refresh(&mut self, list: &ListState) {
        let viewport = list.viewport_bounds().size;
        if self.viewport != viewport || self.heights.len() != list.item_count() {
            self.heights = vec![None; list.item_count()];
            self.viewport = viewport;
            self.drag = None;
        }
        for (index, height) in self
            .heights
            .iter_mut()
            .enumerate()
            .skip(list.logical_scroll_top().item_ix)
        {
            let Some(bounds) = list.bounds_for_item(index) else {
                break;
            };
            *height = Some(bounds.size.height);
        }
    }

    fn extent(&self) -> Extent {
        let (measured, count) = self
            .heights
            .iter()
            .flatten()
            .fold((px(0.0), 0.0_f32), |(total, count), height| {
                (total + *height, count + 1.0)
            });
        let estimate = if count > 0.0 {
            (measured / count).max(px(1.0))
        } else {
            self.viewport.height.max(px(1.0))
        };
        let heights: Vec<_> = self
            .heights
            .iter()
            .map(|height| height.unwrap_or(estimate))
            .collect();
        let total = heights
            .iter()
            .fold(px(0.0), |total, height| total + *height);
        Extent {
            heights,
            total,
            visible: self.viewport.height,
        }
    }

    fn thumb_bounds(&self, list: &ListState, track: Bounds<Pixels>) -> Option<Bounds<Pixels>> {
        if let Some(drag) = &self.drag {
            drag.extent.thumb_bounds(track, drag.offset)
        } else {
            let extent = self.extent();
            extent.thumb_bounds(track, extent.offset(list))
        }
    }
}

impl Extent {
    fn maximum_offset(&self) -> Pixels {
        (self.total - self.visible).max(px(0.0))
    }

    fn offset(&self, list: &ListState) -> Pixels {
        let top = list.logical_scroll_top();
        self.heights
            .iter()
            .take(top.item_ix)
            .fold(top.offset_in_item, |offset, height| offset + *height)
    }

    fn logical_offset(&self, mut offset: Pixels) -> ListOffset {
        if offset < self.maximum_offset() {
            for (item_ix, height) in self.heights.iter().enumerate() {
                if offset < *height {
                    return ListOffset {
                        item_ix,
                        offset_in_item: offset,
                    };
                }
                offset -= *height;
            }
        }
        ListOffset {
            item_ix: self.heights.len(),
            offset_in_item: px(0.0),
        }
    }

    #[allow(
        clippy::cast_possible_truncation,
        reason = "Scrollbar geometry is bounded by GPUI f32 pixels."
    )]
    fn thumb_bounds(&self, track: Bounds<Pixels>, offset: Pixels) -> Option<Bounds<Pixels>> {
        ThumbGeometry::from_lengths(
            f64::from(f32::from(self.total)),
            f64::from(f32::from(self.visible)),
            f64::from(f32::from(offset)),
            f64::from(f32::from(track.size.height)),
            MINIMUM_THUMB_LENGTH,
        )
        .map(|thumb| {
            Bounds::new(
                point(
                    track.left() + px(2.0),
                    track.top() + px(thumb.origin as f32),
                ),
                size(px(6.0), px(thumb.length as f32)),
            )
        })
    }
}

impl Drag {
    fn scroll_to_pointer(&mut self, list: &ListState, track: Bounds<Pixels>, pointer: Pixels) {
        let Some(thumb) = self.extent.thumb_bounds(track, self.offset) else {
            return;
        };
        let travel = track.size.height - thumb.size.height;
        if travel > px(0.0) {
            let fraction =
                (f32::from(pointer - track.top() - self.grab) / f32::from(travel)).clamp(0.0, 1.0);
            self.offset = self.extent.maximum_offset() * fraction;
            list.scroll_to(self.extent.logical_offset(self.offset));
        }
    }
}
