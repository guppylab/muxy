use gpui::prelude::FluentBuilder;
use gpui::{AnyElement, Div, ParentElement, Styled, div, px, relative};

use crate::controls::Style;

pub fn row(
    style: Style,
    label: &str,
    description: Option<&str>,
    control: AnyElement,
    stacked: bool,
) -> Div {
    let Style { theme, metrics } = style;
    let text = div()
        .flex_1()
        .min_w(px(0.0))
        .flex()
        .flex_col()
        .gap(metrics.scaled(5.0))
        .child(
            div()
                .text_size(metrics.font_headline())
                .text_color(theme.fg)
                .child(label.to_owned()),
        )
        .when_some(description, |text, description| {
            text.child(
                div()
                    .text_size(metrics.font_emphasis())
                    .line_height(relative(1.55))
                    .text_color(theme.fg_muted)
                    .child(description.to_owned()),
            )
        });
    div()
        .flex()
        .min_w(px(0.0))
        .py(metrics.scaled(22.0))
        .gap(metrics.spacing9())
        .map(|row| {
            if stacked {
                row.flex_col().gap(metrics.spacing6())
            } else {
                row.items_center()
            }
        })
        .child(text)
        .child(div().flex_none().min_w(px(0.0)).child(control))
}

pub fn section_heading(style: Style, title: &str) -> Div {
    let Style { theme, metrics } = style;
    div()
        .pt(metrics.spacing8())
        .pb(metrics.spacing6())
        .border_b_1()
        .border_color(theme.border)
        .font_family(".SystemUIFontMonospaced")
        .text_size(metrics.font_body())
        .text_color(theme.fg_muted)
        .child(title.to_owned())
}

pub fn note(style: Style, text: &str, error: bool) -> Div {
    let Style { theme, metrics } = style;
    div()
        .py(metrics.spacing7())
        .text_size(metrics.font_footnote())
        .text_color(if error { theme.danger } else { theme.fg_muted })
        .child(text.to_owned())
}
