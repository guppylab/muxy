use gpui::prelude::FluentBuilder;
use gpui::{
    Animation, AnimationExt as _, AnyElement, Bounds, Hsla, IntoElement, ParentElement,
    PathBuilder, Pixels, SharedString, Styled, canvas, div, percentage, point, px, svg,
};
use muxy_app_core::Tab;
use muxy_protocol::{ProgressState, TerminalProgress};

use crate::model::AppModel;
use gpui::InteractiveElement;

pub(super) fn glyph(tab: &Tab, model: &AppModel, size: Pixels, fallback: AnyElement) -> AnyElement {
    let mut progress = None;
    let mut completion = false;
    for pane in &tab.panes {
        if let muxy_app_core::PaneContent::Terminal {
            session: Some(session),
        } = pane.content
        {
            progress = progress.or(model
                .progress
                .get(&session)
                .and_then(|state| state.progress));
            completion |= model.completions.contains(&pane.id);
        }
    }
    let id = tab.id;
    div()
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(size)
        .child(match progress {
            Some(progress) => div()
                .debug_selector(move || format!("tab-progress-{id}"))
                .flex()
                .child(progress_circle(id, progress, size, &model.theme))
                .into_any_element(),
            None => fallback,
        })
        .when(completion, |glyph| {
            glyph.child(
                div()
                    .debug_selector(move || format!("tab-completion-{id}"))
                    .absolute()
                    .top(px(-2.0))
                    .right(px(-2.0))
                    .size(model.metrics.scaled(6.0))
                    .rounded_full()
                    .bg(model.theme.accent),
            )
        })
        .into_any_element()
}

fn progress_circle(
    id: muxy_app_core::TabId,
    progress: TerminalProgress,
    size: Pixels,
    theme: &muxy_ui::theme::Theme,
) -> AnyElement {
    let color = match progress.state {
        ProgressState::Error => theme.danger,
        ProgressState::Paused => theme.warning,
        ProgressState::Running | ProgressState::Indeterminate => theme.accent,
    };
    if progress.state == ProgressState::Indeterminate {
        return svg()
            .path("icons/progress-indeterminate.svg")
            .size(size)
            .text_color(color)
            .with_animation(
                SharedString::from(format!("terminal-progress-{id}")),
                Animation::new(std::time::Duration::from_secs(1)).repeat(),
                |svg, delta| {
                    svg.with_transformation(gpui::Transformation::rotate(percentage(delta)))
                },
            )
            .into_any_element();
    }
    progress_ring(
        size,
        f32::from(progress.percent.unwrap_or(100)) / 100.0,
        color,
    )
}

fn progress_ring(size: Pixels, fraction: f32, color: Hsla) -> AnyElement {
    let line_width = px((f32::from(size) / 8.0).max(1.0));
    canvas(
        move |bounds, _, _| {
            (
                ring_path(bounds, 1.0, line_width),
                ring_path(bounds, fraction.max(0.001), line_width),
            )
        },
        move |_, paths, window, _| {
            if let Some(path) = paths.0 {
                window.paint_path(path, color.opacity(0.25));
            }
            if let Some(path) = paths.1 {
                window.paint_path(path, color);
            }
        },
    )
    .size(size)
    .into_any_element()
}

fn ring_path(
    bounds: Bounds<Pixels>,
    fraction: f32,
    line_width: Pixels,
) -> Option<gpui::Path<Pixels>> {
    let center = bounds.center();
    let radius = (bounds.size.width.min(bounds.size.height) - line_width) / 2.0;
    let mut builder = PathBuilder::stroke(line_width);
    for step in 0..=48_u16 {
        let angle = -std::f32::consts::FRAC_PI_2
            + std::f32::consts::TAU * fraction.clamp(0.0, 1.0) * f32::from(step) / 48.0;
        let point = point(
            center.x + radius * angle.cos(),
            center.y + radius * angle.sin(),
        );
        if step == 0 {
            builder.move_to(point);
        } else {
            builder.line_to(point);
        }
    }
    builder.build().ok()
}
