//! Candidate popup rendering.

use cosmic::iced::{self, Color};
use cosmic::widget::{self, container, row, text};
use cosmic::Element;

use crate::state::InputMethodState;
use crate::Message;

pub fn view<'a>(state: &'a InputMethodState, _window: iced::window::Id) -> Element<'a, Message> {
    // Only paint when there are candidates. Empty states stay 0×0 so a leftover
    // surface size cannot show as a solid cursor-following box.
    if !state.editor.is_selecting() {
        return widget::Space::new().width(0).height(0).into();
    }

    let pages = state.visible_pages();
    if pages.is_empty() {
        return widget::Space::new().width(0).height(0).into();
    }

    let current_page_in_view = state.page.saturating_sub(state.visible_page_offset);
    let cosmic_theme = cosmic::theme::active();
    let cosmic = cosmic_theme.cosmic();
    let spacing = cosmic.spacing;
    let corner_radius = cosmic.corner_radii.radius_xs;
    let vertical = state.config.vertical_lookup_table;

    let colors = CandidateColors {
        selected_bg: Color::from(cosmic.accent_color()),
        selected_fg: Color::from(cosmic.on_accent_color()),
        normal_fg: Color::from(cosmic.primary(false).on),
        dim_fg: Color::from(cosmic.primary(false).component.on_disabled),
    };

    let sel_keys = &state.config.selection_keys;

    let page_blocks: Vec<Element<'_, Message>> = pages
        .iter()
        .enumerate()
        .map(|(page_idx, list)| {
            let is_active_page = page_idx == current_page_in_view;
            let items: Vec<Element<'_, Message>> = list
                .iter()
                .enumerate()
                .map(|(idx, ch)| {
                    let is_selected = is_active_page && idx == state.index;
                    candidate_item(
                        ch,
                        idx,
                        is_active_page,
                        is_selected,
                        sel_keys,
                        &colors,
                        spacing.space_s,
                        spacing.space_xxxs,
                        [spacing.space_xxs, spacing.space_xs],
                        corner_radius,
                    )
                })
                .collect();

            if vertical {
                widget::column(items)
                    .spacing(spacing.space_xxxs)
                    .into()
            } else {
                row(items).spacing(spacing.space_xxxs).into()
            }
        })
        .collect();

    let candidates: Element<'_, Message> = if vertical {
        // Vertical lists side-by-side when multiple pages are visible.
        row(page_blocks).spacing(spacing.space_xxs).into()
    } else {
        // Horizontal lists stacked when multiple pages are visible.
        widget::column(page_blocks)
            .spacing(spacing.space_xxs)
            .into()
    };

    let body: Element<'_, Message> = if state.config.show_page_number {
        let total = state.total_pages().max(1);
        let page_label = format!("{} / {}", state.page + 1, total);
        widget::column![
            candidates,
            text::caption(page_label).class(cosmic::theme::style::Text::Color(colors.dim_fg)),
        ]
        .spacing(spacing.space_xxs)
        .into()
    } else {
        candidates
    };

    cosmic::widget::autosize::autosize(
        container(body)
            .padding(spacing.space_xxs)
            .width(iced::Length::Shrink)
            .height(iced::Length::Shrink)
            .class(cosmic::theme::Container::Dropdown),
        cosmic::widget::Id::new("im-popup"),
    )
    .into()
}

struct CandidateColors {
    selected_bg: Color,
    selected_fg: Color,
    normal_fg: Color,
    dim_fg: Color,
}

fn candidate_item<'a>(
    text_str: &str,
    index: usize,
    show_number: bool,
    is_selected: bool,
    selection_keys: &str,
    colors: &CandidateColors,
    number_width: u16,
    item_spacing: u16,
    padding: [u16; 2],
    corner_radius: [f32; 4],
) -> Element<'a, Message> {
    let num_color = if is_selected {
        colors.selected_fg
    } else {
        colors.dim_fg
    };
    let text_color = if is_selected {
        colors.selected_fg
    } else {
        colors.normal_fg
    };

    let number_text = if show_number {
        selection_keys
            .chars()
            .nth(index)
            .map(|c| c.to_string())
            .unwrap_or_else(|| format!("{}", (index + 1) % 10))
    } else {
        String::new()
    };

    let content = row![
        container(text::body(number_text).class(cosmic::theme::style::Text::Color(num_color)))
            .width(number_width)
            .align_x(iced::alignment::Horizontal::Right),
        text::body(text_str.to_string()).class(cosmic::theme::style::Text::Color(text_color)),
    ]
    .align_y(iced::Alignment::Center)
    .spacing(item_spacing);

    if is_selected {
        let bg = colors.selected_bg;
        container(content)
            .padding(padding)
            .class(cosmic::theme::Container::custom(move |_| {
                container::Style {
                    background: Some(iced::Background::Color(bg)),
                    border: iced::Border {
                        radius: corner_radius.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            }))
            .into()
    } else {
        container(content).padding(padding).into()
    }
}
