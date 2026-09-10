use super::{Category, SettingsPane, appearance, keyboard, server, terminal};
use gpui::{
    AnyElement, Context, FocusHandle, InteractiveElement, IntoElement, ListAlignment, ListState,
    ParentElement, Pixels, Styled, Window, div, px,
};
use muxy_ui::controls;
use std::collections::HashMap;

pub(super) struct Results {
    pub(super) state: ListState,
    items: Vec<Item>,
    focus: HashMap<Category, FocusHandle>,
    pub(super) dirty: bool,
    reset_scroll: bool,
    overdraw: Pixels,
}

#[derive(Clone, Copy)]
enum Item {
    Section(Category),
    KeyboardHeading,
    Shortcut(usize),
    Divider,
    Empty,
    Footer,
}

impl Results {
    pub(super) fn new(cx: &mut Context<SettingsPane>) -> Self {
        Self {
            state: ListState::new(0, ListAlignment::Top, px(0.0)),
            items: Vec::new(),
            focus: Category::ALL
                .into_iter()
                .map(|category| (category, cx.focus_handle()))
                .collect(),
            dirty: true,
            reset_scroll: true,
            overdraw: px(0.0),
        }
    }

    pub(super) fn reset(&mut self) {
        self.dirty = true;
        self.reset_scroll = true;
    }
}

impl SettingsPane {
    pub(super) fn refresh_results(
        &mut self,
        overdraw: Pixels,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if !self.results.dirty && self.results.overdraw == overdraw {
            return;
        }
        let mut items = Vec::new();
        for category in Category::ALL {
            if self.query.is_empty() && category != self.category {
                continue;
            }
            if category == Category::Keyboard {
                let shortcuts = keyboard::matching(self);
                if !shortcuts.is_empty() {
                    items.push(Item::KeyboardHeading);
                    items.extend(shortcuts.into_iter().map(Item::Shortcut));
                    items.push(Item::Divider);
                }
            } else if !self.section_rows(category, window, cx).is_empty() {
                items.push(Item::Section(category));
            }
        }
        if items.is_empty() {
            items.push(Item::Empty);
        }
        items.push(Item::Footer);
        let offset = self.results.state.logical_scroll_top();
        if self.results.overdraw == overdraw {
            self.results.state.reset(items.len());
        } else {
            self.results.state = ListState::new(items.len(), ListAlignment::Top, overdraw);
            self.results.overdraw = overdraw;
        }
        self.results.state.splice_focusable(
            0..items.len(),
            items.iter().map(|item| match item {
                Item::Section(category) => Some(self.results.focus[category].clone()),
                _ => None,
            }),
        );
        if !self.results.reset_scroll {
            self.results.state.scroll_to(offset);
        }
        self.results.items = items;
        self.results.dirty = false;
        self.results.reset_scroll = false;
    }

    fn section_rows(
        &self,
        category: Category,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        match category {
            Category::Appearance => appearance::rows(self, cx),
            Category::Terminal => terminal::rows(self, window, cx),
            Category::Server => server::rows(self, cx),
            Category::Keyboard => Vec::new(),
        }
    }

    pub(super) fn result(
        &mut self,
        index: usize,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match self.results.items[index] {
            Item::Section(category) => div()
                .w_full()
                .min_w(px(0.0))
                .track_focus(&self.results.focus[&category])
                .debug_selector(move || format!("settings-section-{}", category.label()))
                .child(controls::section(
                    self.style(),
                    category.label(),
                    None,
                    true,
                    self.section_rows(category, window, cx),
                ))
                .into_any_element(),
            Item::KeyboardHeading => div()
                .w_full()
                .debug_selector(|| "settings-section-Keyboard".into())
                .child(controls::section(self.style(), "Keyboard", None, false, Vec::new()))
                .child(self.note("Click a shortcut to record it. Escape cancels. Conflicts in the same context must be resolved first.", false))
                .into_any_element(),
            Item::Shortcut(index) => {
                #[cfg(test)]
                { self.shortcut_row_count += 1; }
                keyboard::row(self, index, cx)
            },
            Item::Divider => div()
                .mx(self.metrics.spacing6())
                .h(px(1.0))
                .bg(self.theme.border)
                .into_any_element(),
            Item::Empty => div()
                .w_full()
                .debug_selector(|| "settings-empty".into())
                .pt(self.metrics.spacing5())
                .child(self.note("No settings found. Try another search.", false))
                .into_any_element(),
            Item::Footer => self.note(
                "Changes apply immediately. Press Return or leave a field to save text.",
                false,
            ),
        }
    }
}
