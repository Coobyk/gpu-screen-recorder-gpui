//! Small, reusable, stateless widgets built on GPUI's `div` elements.

use gpui::{
    AnyElement, App, ClickEvent, FontWeight, IntoElement, SharedString, Window, div, prelude::*, px,
};

use crate::theme;

pub fn label(text: impl Into<SharedString>) -> AnyElement {
    div()
        .text_sm()
        .text_color(theme::TEXT)
        .child(text.into())
        .into_any_element()
}

pub fn muted(text: impl Into<SharedString>) -> AnyElement {
    div()
        .text_sm()
        .text_color(theme::TEXT_MUTED)
        .child(text.into())
        .into_any_element()
}

pub fn heading(text: impl Into<SharedString>) -> AnyElement {
    div()
        .text_lg()
        .font_weight(FontWeight::BOLD)
        .text_color(theme::TEXT)
        .child(text.into())
        .into_any_element()
}

/// A titled, bordered group of controls.
pub fn section(title: impl Into<SharedString>, children: Vec<AnyElement>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .bg(theme::PANEL)
        .border_1()
        .border_color(theme::BORDER)
        .rounded_md()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .text_color(theme::TEXT_MUTED)
                .child(title.into()),
        )
        .children(children)
        .into_any_element()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Primary,
    Secondary,
    Danger,
}

impl ButtonKind {
    fn bg(self) -> gpui::Rgba {
        match self {
            ButtonKind::Primary => theme::ACCENT,
            ButtonKind::Secondary => theme::PANEL_ALT,
            ButtonKind::Danger => theme::DANGER,
        }
    }

    fn hover_bg(self) -> gpui::Rgba {
        match self {
            ButtonKind::Primary => theme::ACCENT_HOVER,
            ButtonKind::Secondary => theme::BORDER,
            ButtonKind::Danger => theme::DANGER_HOVER,
        }
    }
}

pub fn button(
    id: impl Into<SharedString>,
    text: impl Into<SharedString>,
    kind: ButtonKind,
    enabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let bg = kind.bg();
    let hover_bg = kind.hover_bg();
    let text_color = if enabled {
        theme::TEXT
    } else {
        theme::TEXT_MUTED
    };
    let element = div()
        .id(id.into())
        .flex()
        .items_center()
        .justify_center()
        .h(px(32.))
        .px_3()
        .rounded_md()
        .border_1()
        .border_color(theme::BORDER)
        .bg(bg)
        .text_color(text_color)
        .text_sm()
        .child(text.into());
    let element = if enabled {
        element
            .cursor_pointer()
            .hover(move |style| style.bg(hover_bg))
            .on_click(on_click)
    } else {
        element.opacity(0.5)
    };
    element.into_any_element()
}

pub fn checkbox(
    id: impl Into<SharedString>,
    text: impl Into<SharedString>,
    checked: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id.into())
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .w(px(16.))
                .h(px(16.))
                .rounded_sm()
                .border_1()
                .border_color(if checked { theme::ACCENT } else { theme::BORDER })
                .bg(if checked { theme::ACCENT } else { theme::FIELD })
                .text_xs()
                .text_color(theme::TEXT)
                .child(if checked { "\u{2713}" } else { "" }),
        )
        .child(
            div()
                .text_sm()
                .text_color(theme::TEXT)
                .child(text.into()),
        )
        .on_click(on_click)
        .into_any_element()
}

pub fn separator() -> AnyElement {
    div()
        .h(px(1.))
        .w_full()
        .bg(theme::BORDER)
        .into_any_element()
}
