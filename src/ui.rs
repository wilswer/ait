use std::env;

use pathdiff::diff_paths;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Flex, Layout, Margin, Rect},
    style::{
        Color::{self, DarkGray},
        Modifier, Style, Stylize,
    },
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Borders, Clear, FrameExt, HighlightSpacing, List, ListItem, Padding,
        Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
    },
};
use syntect::highlighting::Theme;
use tui_big_text::{BigText, PixelSize};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    app::{App, AppMode, Message, Notification, THINKING_EFFORTS, get_file_content},
    snippets::{
        MessageSegment, MessageText, create_highlighted_code, parse_message_segments,
        translate_language_name_to_syntect_name,
    },
    storage::list_all_messages,
};
const SPINNER_FRAMES: &[&str] = &[
    " ⠀⠀",
    "⡀⠀⠀",
    "⡄⠀⠀",
    "⡆⠀⠀",
    "⡇⠀⠀",
    "⣇⠀⠀",
    "⣧⠀⠀",
    "⣷⠀⠀",
    "⣿⠀⠀",
    "⣿⡀⠀",
    "⣿⡄⠀",
    "⣿⡆⠀",
    "⣿⡇⠀",
    "⣿⣇⠀",
    "⣿⣧⠀",
    "⣿⣷⠀",
    "⣿⣿⠀",
    "⣿⣿⡀",
    "⣿⣿⡄",
    "⣿⣿⡆",
    "⣿⣿⡇",
    "⣿⣿⣇",
    "⣿⣿⣧",
    "⣿⣿⣷",
    "⣿⣿⣿", // Midway
    "⣿⣿⣿", // Midway
    "⣾⣿⣿",
    "⣼⣿⣿",
    "⣸⣿⣿",
    "⢸⣿⣿",
    "⢰⣿⣿",
    "⢠⣿⣿",
    "⢀⣿⣿",
    "⠀⣿⣿",
    "⠀⣾⣿",
    "⠀⣼⣿",
    "⠀⣸⣿",
    "⠀⢸⣿",
    "⠀⢰⣿",
    "⠀⢠⣿",
    "⠀⢀⣿",
    "⠀⠀⣿",
    "⠀⠀⣾",
    "⠀⠀⣼",
    "⠀⠀⣸",
    "⠀⠀⢸",
    "⠀⠀⢰",
    "⠀⠀⢠",
    "⠀⠀⢀",
    "⠀⠀ ",
    "⠀⠀ ",
    "⠀⠀⢀",
    "⠀⠀⢠",
    "⠀⠀⢰",
    "⠀⠀⢸",
    "⠀⠀⣸",
    "⠀⠀⣼",
    "⠀⠀⣾",
    "⠀⠀⣿",
    "⠀⢀⣿",
    "⠀⢠⣿",
    "⠀⢰⣿",
    "⠀⢸⣿",
    "⠀⣸⣿",
    "⠀⣼⣿",
    "⠀⣾⣿",
    "⠀⣿⣿",
    "⢀⣿⣿",
    "⢠⣿⣿",
    "⢰⣿⣿",
    "⢸⣿⣿",
    "⣸⣿⣿",
    "⣼⣿⣿",
    "⣾⣿⣿",
    "⣿⣿⣿", // Midway
    "⣿⣿⣿", // Midway
    "⣿⣿⣷",
    "⣿⣿⣧",
    "⣿⣿⣇",
    "⣿⣿⡇",
    "⣿⣿⡆",
    "⣿⣿⡄",
    "⣿⣿⡀",
    "⣿⣿⠀",
    "⣿⣷⠀",
    "⣿⣧⠀",
    "⣿⣇⠀",
    "⣿⡇⠀",
    "⣿⡆⠀",
    "⣿⡄⠀",
    "⣿⡀⠀",
    "⣿⠀⠀",
    "⣷⠀⠀",
    "⣧⠀⠀",
    "⣇⠀⠀",
    "⡇⠀⠀",
    "⡆⠀⠀",
    "⡄⠀⠀",
    "⡀⠀⠀",
    " ⠀⠀",
];
const THINKING_VERB: &str = "Processing user query... ";

pub const SELECTED_STYLE: Style = Style::new()
    .add_modifier(Modifier::BOLD)
    .fg(Color::LightBlue)
    .bg(Color::DarkGray);

/// helper function to create a centered rect using up certain percentage of the available rect `r`
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(r);

    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(popup_layout[1])[1]
}

fn centered_rects_with_search(percent_x: u16, percent_y: u16, r: Rect) -> (Rect, Rect) {
    let popup_layout = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Length(3),
        Constraint::Fill(1),
    ])
    .split(r);

    let main_rect = Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(popup_layout[1])[1];
    let search_rect = Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(popup_layout[2])[1];
    (main_rect, search_rect)
}

fn right_aligned_rect(r: Rect, p: u16) -> Rect {
    Layout::horizontal([Constraint::Percentage(100 - p), Constraint::Fill(1)]).split(r)[1]
}

fn left_aligned_rect(r: Rect, p: u16) -> Rect {
    Layout::horizontal([Constraint::Fill(1), Constraint::Percentage(100 - p)]).split(r)[0]
}

/// Parse a single line for inline markdown markers (`**bold**`, `*italic*`, `` `code` ``).
/// Returns a vec of styled [`Span`]s.
fn parse_inline_markdown(text: &str, style: Style) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut current = String::new();
    let mut rest = text;

    while !rest.is_empty() {
        if rest.starts_with("**")
            && let Some(end) = rest[2..].find("**")
        {
            if !current.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut current), style));
            }
            spans.push(Span::styled(
                rest[2..2 + end].to_string(),
                style.patch(Style::default().bold()),
            ));
            rest = &rest[2 + end + 2..];
            continue;
        }
        if rest.starts_with('*') {
            // single star italic — only if there is a closing *
            if let Some(end) = rest[1..].find('*') {
                let inner = &rest[1..1 + end];
                if !inner.is_empty() && !inner.contains('\n') {
                    if !current.is_empty() {
                        spans.push(Span::styled(std::mem::take(&mut current), style));
                    }
                    spans.push(Span::styled(
                        inner.to_string(),
                        style.patch(Style::default().italic()),
                    ));
                    rest = &rest[1 + end + 1..];
                    continue;
                }
            }
        }
        if rest.starts_with('`')
            && let Some(end) = rest[1..].find('`')
        {
            let inner = &rest[1..1 + end];
            if !inner.is_empty() {
                if !current.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut current), style));
                }
                spans.push(Span::styled(
                    inner.to_string(),
                    style.patch(Style::default().fg(Color::Yellow)),
                ));
                rest = &rest[1 + end + 1..];
                continue;
            }
        }
        let c = rest.chars().next().unwrap();
        current.push(c);
        rest = &rest[c.len_utf8()..];
    }

    if !current.is_empty() {
        spans.push(Span::styled(current, style));
    }
    spans
}

fn is_separator(s: &str) -> bool {
    s.len() >= 3
        && (s.chars().all(|c| c == '-')
            || s.chars().all(|c| c == '=')
            || s.chars().all(|c| c == '*'))
}

/// Render a markdown text segment into styled [`Line`]s, with word-wrapping.
fn render_markdown_lines(text: &str, width: usize, style: Style) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();

        if trimmed.is_empty() {
            lines.push(Line::default());
            continue;
        }

        // Horizontal rule
        if is_separator(trimmed) {
            lines.push(
                Line::from("─".repeat(3)).style(style.patch(Style::default().fg(Color::DarkGray))),
            );
            continue;
        }

        // ATX headings: # / ## / ###
        if trimmed.starts_with('#') {
            let level = trimmed.chars().take_while(|&c| c == '#').count().min(6);
            let heading_text = trimmed[level..].trim();
            let heading_style = match level {
                1 => style.patch(Style::default().bold().fg(Color::Blue)),
                2 => style.patch(Style::default().bold().fg(Color::Magenta)),
                3 => style.patch(Style::default().bold().fg(Color::Cyan)),
                4 => style.patch(Style::default().bold().fg(Color::LightBlue)),
                5 => style.patch(Style::default().bold().fg(Color::LightMagenta)),
                6 => style.patch(Style::default().bold().fg(Color::LightCyan)),
                _ => style.patch(Style::default().bold()),
            };
            let prefix = format!("{} ", "#".repeat(level));
            let mut spans = vec![Span::styled(prefix, heading_style)];
            for s in parse_inline_markdown(heading_text, heading_style) {
                spans.push(Span::styled(
                    s.content.into_owned(),
                    heading_style.patch(s.style),
                ));
            }
            lines.push(Line::from(spans));
            continue;
        }

        // Unordered list item: - / * / +
        let is_unordered =
            trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ");
        if is_unordered {
            let item_text = &trimmed[2..];
            let bullet_prefix = format!("{}• ", " ".repeat(indent));
            let prefix_w = bullet_prefix.chars().count();
            let avail = width.saturating_sub(prefix_w).max(1);
            for (i, piece) in textwrap::wrap(item_text, avail).iter().enumerate() {
                let mut spans = if i == 0 {
                    vec![Span::styled(
                        bullet_prefix.clone(),
                        style.patch(Style::default().fg(Color::DarkGray)),
                    )]
                } else {
                    vec![Span::styled(" ".repeat(prefix_w), style)]
                };
                spans.extend(parse_inline_markdown(piece, style));
                lines.push(Line::from(spans));
            }
            continue;
        }

        // Ordered list item: 1. / 12. etc.
        let num_end = trimmed.find(". ").unwrap_or(0);
        let is_ordered = num_end > 0 && trimmed[..num_end].chars().all(|c| c.is_ascii_digit());
        if is_ordered {
            let num_prefix = format!("{}{}. ", " ".repeat(indent), &trimmed[..num_end]);
            let prefix_w = num_prefix.chars().count();
            let item_text = &trimmed[num_end + 2..];
            let avail = width.saturating_sub(prefix_w).max(1);
            for (i, piece) in textwrap::wrap(item_text, avail).iter().enumerate() {
                let mut spans = if i == 0 {
                    vec![Span::styled(
                        num_prefix.clone(),
                        style.patch(Style::default().fg(Color::DarkGray)),
                    )]
                } else {
                    vec![Span::styled(" ".repeat(prefix_w), style)]
                };
                spans.extend(parse_inline_markdown(piece, style));
                lines.push(Line::from(spans));
            }
            continue;
        }

        // Regular paragraph text (word-wrapped to the available width)
        for piece in textwrap::wrap(line, width.max(1)) {
            lines.push(Line::from(parse_inline_markdown(&piece, style)));
        }
    }

    lines
}

fn process_code_blocks<'a>(text: impl Into<String>, width: usize, theme: Theme) -> Vec<Line<'a>> {
    let mut lines = Vec::new();
    let text = text.into();
    let style = Style::default();
    for segment in parse_message_segments(&text) {
        match segment {
            MessageSegment::Text(MessageText {
                text: mtext,
                is_thought,
            }) => {
                let style = if is_thought {
                    style.patch(Style::default().dim().italic())
                } else {
                    style
                };
                lines.extend(render_markdown_lines(&mtext, width, style));
            }
            MessageSegment::Code {
                language,
                code,
                indent,
                depth: 0,
                is_thought,
            } => {
                let style = if is_thought {
                    style.patch(Style::default().dim().italic())
                } else {
                    style
                };
                if !code.is_empty() {
                    let mut code_lines = Vec::new();
                    code_lines.push(
                        Line::from(format!("{}```{}", " ".repeat(indent), &language))
                            .style(style.patch(Style::default().fg(Color::DarkGray))),
                    );
                    let clines = if !language.is_empty() {
                        create_highlighted_code(
                            &code,
                            translate_language_name_to_syntect_name(Some(&language)),
                            &theme,
                            style,
                        )
                    } else {
                        let wrapped = textwrap::wrap(&code, width);
                        wrapped
                            .into_iter()
                            .map(|l| Line::from(Span::raw(l.into_owned())))
                            .collect()
                    };
                    code_lines.extend(clines);
                    code_lines.push(
                        Line::from(format!("{}```", " ".repeat(indent)))
                            .style(style.patch(Style::default().fg(Color::DarkGray))),
                    );
                    lines.extend(code_lines);
                }
            }
            // Nested blocks (depth > 0) are already embedded verbatim in the
            // outer block's syntax-highlighted content; skip them here.
            MessageSegment::Code { .. } => {}
        }
    }
    lines
}

/// Percentage of the available line width a bubble may occupy at most.
const BUBBLE_MAX_PERCENT: usize = 100;

#[derive(Clone, Copy)]
enum BubbleAlign {
    Left,
    Right,
}

struct BubbleSkin {
    title: &'static str,
    align: BubbleAlign,
    border: Style,
}

fn user_skin() -> BubbleSkin {
    BubbleSkin {
        title: "User",
        align: BubbleAlign::Right,
        border: Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    }
}

fn assistant_skin() -> BubbleSkin {
    BubbleSkin {
        title: "Assistant",
        align: BubbleAlign::Left,
        border: Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    }
}

/// Maximum width available for the *content* (text) inside a bubble, given the
/// total width available for a rendered line.
fn bubble_max_content_width(line_width: usize) -> usize {
    let max_outer = line_width * BUBBLE_MAX_PERCENT / 100;
    max_outer.saturating_sub(4 + 4)
}

/// Clip the given line to `width` display columns (preserving span styles) and
/// pad it with spaces so the resulting spans are exactly `width` columns wide.
fn fit_spans<'a>(line: &Line, width: usize) -> Vec<Span<'a>> {
    let mut out: Vec<Span<'a>> = Vec::new();
    let mut used = 0usize;
    for span in &line.spans {
        if used >= width {
            break;
        }
        let style = line.style.patch(span.style);
        let remaining = width - used;
        let content = span.content.as_ref();
        if UnicodeWidthStr::width(content) <= remaining {
            used += UnicodeWidthStr::width(content);
            out.push(Span::styled(content.to_string(), style));
        } else {
            let mut s = String::new();
            let mut c = 0usize;
            for ch in content.chars() {
                let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
                if c + cw > remaining {
                    break;
                }
                s.push(ch);
                c += cw;
            }
            used += c;
            out.push(Span::styled(s, style));
        }
    }
    if used < width {
        out.push(Span::raw(" ".repeat(width - used)));
    }
    out
}

/// Wrap already-rendered body lines in a rounded chat bubble, aligned left or
/// right within `line_width` columns.
fn frame_bubble<'a>(body: Vec<Line<'a>>, line_width: usize, skin: &BubbleSkin) -> Vec<Line<'a>> {
    let max_content = bubble_max_content_width(line_width);
    let content_width = body
        .iter()
        .map(|l| l.width())
        .max()
        .unwrap_or(0)
        .min(max_content)
        .max(skin.title.len() + 1)
        .min(line_width.saturating_sub(4).max(1));

    let outer = content_width + 4;
    let indent = match skin.align {
        BubbleAlign::Left => 0,
        BubbleAlign::Right => line_width.saturating_sub(outer),
    };
    let pad = |spans: Vec<Span<'a>>| -> Line<'a> {
        if indent > 0 {
            let mut v = vec![Span::raw(" ".repeat(indent))];
            v.extend(spans);
            Line::from(v)
        } else {
            Line::from(spans)
        }
    };

    let mut lines: Vec<Line<'a>> = Vec::new();

    if skin.title == "Assistant" {
        // Top border: ╭─ Assistant ───────╮
        let head = format!("╭─ {} ", skin.title);
        let fill = outer.saturating_sub(head.chars().count() + 1);
        lines.push(pad(vec![Span::styled(
            format!("{}{}╮", head, "─".repeat(fill)),
            skin.border,
        )]));
    } else {
        // Top border: ╭─────── User ─╮
        let head = format!(" {} ─╮", skin.title);
        let fill = outer.saturating_sub(head.chars().count() + 1);
        lines.push(pad(vec![Span::styled(
            format!("╭{}{}", "─".repeat(fill), head),
            skin.border,
        )]));
    }

    // Body
    for line in &body {
        let mut spans = vec![Span::styled("│ ", skin.border)];
        spans.extend(fit_spans(line, content_width));
        spans.push(Span::styled(" │", skin.border));
        lines.push(pad(spans));
    }

    // Bottom border: ╰──────────────╯
    lines.push(pad(vec![Span::styled(
        format!("╰{}╯", "─".repeat(outer.saturating_sub(2))),
        skin.border,
    )]));

    lines
}

/// Render a single message as a styled (syntax-highlighted) chat bubble.
pub fn style_message<'a>(message: Message, line_width: usize, theme: Theme) -> Vec<Line<'a>> {
    let content_width = bubble_max_content_width(line_width);
    let (skin, text) = match &message {
        Message::User(_) => (user_skin(), message.to_string()),
        Message::Assistant(t) => {
            if t.is_empty() {
                return Vec::new();
            }
            (assistant_skin(), t.clone())
        }
    };
    let body = process_code_blocks(text, content_width, theme);
    let mut lines = frame_bubble(body, line_width, &skin);
    lines.push(Line::from(""));
    lines
}

/// Render an assistant "waiting for response" bubble with an animated spinner.
fn waiting_bubble<'a>(line_width: usize, spinner_frame: usize) -> Vec<Line<'a>> {
    let frame = SPINNER_FRAMES[spinner_frame % SPINNER_FRAMES.len()];
    let thinking_split_n = (spinner_frame / 4) % THINKING_VERB.len();
    let (think1, think2) = THINKING_VERB.split_at(thinking_split_n);
    let body = vec![
        Line::from(vec![
            Span::raw(format!("{frame} ")),
            Span::raw(think1.to_string()).bold(),
            Span::raw(think2.to_string()).dim(),
        ])
        .style(Style::default().fg(Color::DarkGray)),
    ];
    let mut lines = frame_bubble(body, line_width, &assistant_skin());
    lines.push(Line::from(""));
    lines
}

/// Render all messages as plain (non-highlighted) chat bubbles.
pub fn messages_to_lines<'a>(messages: &[Message], line_width: usize) -> Vec<Line<'a>> {
    let content_width = bubble_max_content_width(line_width);
    let mut line_vec = Vec::new();
    for message in messages {
        let (skin, text) = match message {
            Message::User(_) => (user_skin(), message.to_string()),
            Message::Assistant(m) => {
                if m.is_empty() {
                    continue;
                }
                (assistant_skin(), m.clone())
            }
        };
        let body: Vec<Line> = textwrap::wrap(&text, content_width.max(1))
            .into_iter()
            .map(|l| Line::from(Span::raw(l.into_owned())))
            .collect();
        line_vec.extend(frame_bubble(body, line_width, &skin));
        line_vec.push(Line::from(""));
    }
    line_vec
}

fn render_messages(f: &mut Frame, app: &mut App, messages_area: Rect) {
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("↑"))
        .end_symbol(Some("↓"));
    // Width available for a rendered line inside the bordered chat block.
    let line_width = messages_area.width.saturating_sub(2) as usize;
    let mut messages = if !app.is_streaming && app.do_highlight {
        app.cached_lines.clone()
    } else {
        messages_to_lines(&app.messages, line_width)
    };

    if app.is_waiting_for_response {
        messages.extend(waiting_bubble(line_width, app.spinner_frame));
    }

    let mut scrollbar_state = ScrollbarState::new(messages.len()).position(app.vertical_scroll);

    let messages_text = Text::from(messages);
    let messages = Paragraph::new(messages_text)
        .scroll((app.vertical_scroll as u16, 0))
        .block(Block::bordered().title(format!(
            "Chat - {} [effort: {}]",
            app.selected_model_name,
            app.thinking_effort.as_str()
        )));

    f.render_widget(messages, messages_area);

    f.render_stateful_widget(
        scrollbar,
        messages_area.inner(Margin {
            // using an inner vertical margin of 1 unit makes the scrollbar inside the block
            vertical: 1,
            horizontal: 0,
        }),
        &mut scrollbar_state,
    );
}

// Fron Ratatui's website
fn center_rect(area: Rect, horizontal: Constraint, vertical: Constraint) -> Rect {
    let [area] = Layout::horizontal([horizontal])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::vertical([vertical]).flex(Flex::Center).areas(area);
    area
}

fn render_init_screen(f: &mut Frame, area: Rect) {
    let big_text = BigText::builder()
        .alignment(Alignment::Center)
        .pixel_size(PixelSize::Full)
        .lines(vec!["AIT".into()])
        .build();
    let centered_area = center_rect(area, Constraint::Length(24), Constraint::Length(8)); // 3 8x8 characters
    f.render_widget(big_text, centered_area);
}

pub fn render(f: &mut Frame, app: &mut App) {
    let title = format!("AI in the Terminal (AIT v{})", env!("CARGO_PKG_VERSION"));
    let main_block = Block::bordered()
        .title(title)
        .title_alignment(Alignment::Center)
        .border_type(BorderType::Rounded);
    match app.app_mode {
        AppMode::Normal => {
            f.render_widget(main_block.border_style(Style::new().blue()), f.area());
        }
        AppMode::Editing => {
            f.render_widget(main_block.border_style(Style::new().yellow()), f.area());
        }
        _ => {
            f.render_widget(main_block, f.area());
        }
    }

    let input_area_constraint = match app.app_mode {
        AppMode::Editing => Constraint::Min(1),
        _ => Constraint::Length(0),
    };

    let searchbar_constraint = match app.app_mode {
        AppMode::FilterHistory => Constraint::Length(3),
        _ => Constraint::Length(0),
    };

    let vertical = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        input_area_constraint,
        searchbar_constraint,
    ]);

    let vertical = vertical.margin(1);

    let [help_area, messages_area, input_area, searchbar_area] = vertical.areas(f.area());

    match &app.app_mode {
        AppMode::Normal => {
            if !app.messages.is_empty() {
                render_messages(f, app, messages_area);
            } else {
                render_init_screen(f, messages_area);
            }
        }
        AppMode::Editing => {
            render_messages(f, app, messages_area);
            f.render_widget(&app.input_textarea, input_area);
        }
        AppMode::ModelSelection => {
            let (area, _) = centered_rects_with_search(40, 50, messages_area);
            render_popup(f, "Select Model", area);
            render_model_list(f, area, app);
        }
        AppMode::FilterModels => {
            let (area, search_area) = centered_rects_with_search(40, 50, messages_area);
            render_popup(f, "Select Model", area);
            render_model_list(f, area, app);
            f.render_widget(&app.search_bar, search_area);
        }
        AppMode::ThinkingEffortSelection => {
            let area = centered_rect(30, 30, messages_area);
            render_popup(f, "Select Thinking Effort", area);
            render_thinking_effort_list(f, area, app);
        }
        AppMode::SnippetSelection => {
            let area = left_aligned_rect(messages_area, 25);
            render_popup(f, "Select Snippet", area);
            render_snippet_list(f, area, app);

            let preview_area = right_aligned_rect(messages_area, 75);
            render_popup(f, "Snippet Preview", preview_area);
            if let Some(snippet) = app.get_snippet() {
                let snippet_text = if let Some(lang) = &snippet.language {
                    Text::from(create_highlighted_code(
                        &snippet.text,
                        lang,
                        &app.theme,
                        Style::default(),
                    ))
                } else {
                    Text::from(snippet.text.as_str()).magenta()
                };
                f.render_widget(
                    Paragraph::new(snippet_text).block(Block::new().padding(Padding::uniform(1))),
                    preview_area,
                );
            }
        }
        AppMode::ShowHistory => {
            render_chat_history_panel(f, messages_area, app);
        }
        AppMode::FilterHistory => {
            render_chat_history_panel(f, messages_area, app);
            f.render_widget(&app.search_bar, searchbar_area);
        }
        AppMode::Help => {
            let area = centered_rect(50, 60, messages_area);
            render_popup(f, "Help - Use j/k or Up/Down to scroll", area);

            let normal_keys = vec![
                "Press ".into(),
                "Esc/q".bold(),
                " to exit, ".into(),
                "i".bold(),
                " to start editing, ".into(),
                "y".bold(),
                " to copy the last answer (not linux yet), ".into(),
                "m".bold(),
                " to choose model, ".into(),
                "h".bold(),
                " to browse previous conversations, ".into(),
                "s".bold(),
                " to browse code snippets, ".into(),
                "r".bold(),
                " to toggle syntax highlighting, ".into(),
                "t".bold(),
                " to select the next highlighting theme, ".into(),
                "SHIFT + t (T)".bold(),
                " to select the next highlighting theme, ".into(),
                "f".bold(),
                " to explore files, ".into(),
                "c".bold(),
                " to view context files, ".into(),
                "n".bold(),
                " to start a new chat, ".into(),
                "u".bold(),
                " to interrupt the message currently being received, ".into(),
                "CONTROL + r (C-r)".bold(),
                " to redo last message. ".into(),
                "Scroll with ".into(),
                "j/k or Up/Down".bold(),
                ", ".into(),
                "g".bold(),
                " for top, ".into(),
                "G".bold(),
                " for bottom.".into(),
            ];
            let editing_keys = vec![
                "Press ".into(),
                "Esc".bold(),
                " to stop editing. Press ".into(),
                "CONTROL + s (C-s)".bold(),
                " to submit the message. ".into(),
                "Paste into the text area by pressing ".into(),
                "CONTROL + v (C-v)".bold(),
            ];
            let model_keys = vec![
                "Press ".into(),
                "Up/Down".bold(),
                " to select model, or press ".into(),
                "/".bold(),
                " to search models by name, or press ".into(),
                "Enter".bold(),
                " to select model, which immediately enters 'editing' mode.".into(),
            ];
            let chat_keys = vec![
                "Press ".into(),
                "Up/Down".bold(),
                " to select chat, or press ".into(),
                "/".bold(),
                " to search chats by message content, or press ".into(),
                "CONTROL + r (C-r)".bold(),
                " to delete the selected chat, or press ".into(),
                "Enter".bold(),
                " to select a chat, and return to 'normal' mode.".into(),
            ];
            let snippet_keys = vec![
                "Press ".into(),
                "Up/Down".bold(),
                " to select snippet, or press ".into(),
                "Enter".bold(),
                " to copy snippet to the clipboard (not linux yet), and return to 'normal' mode."
                    .into(),
            ];
            let file_explorer_keys = vec![
                "Press ".into(),
                "h/j/k/l or arrows".bold(),
                " to navigate directories and files. Press ".into(),
                "Enter".bold(),
                " to add a file to context. Press ".into(),
                "d".bold(),
                " to remove the selected file from context. Press ".into(),
                "Esc/q".bold(),
                " to return to 'normal' mode.".into(),
            ];
            let context_keys = vec![
                "Files added to context will be automatically included in your next message to the LLM. Press ".into(),
                "Esc/q/Enter".bold(),
                " to return to 'normal' mode.".into(),
            ];
            let msg = vec![
                Line::from(Span::raw("Welcome to AI in the Terminal! ").bold()),
                Line::from(""),
                Line::from(vec![
                    "When in ".bold(),
                    "normal".bold().blue(),
                    " mode, you can:".bold(),
                ]),
                Line::from(normal_keys),
                Line::from(""),
                Line::from(vec![
                    "When in ".bold(),
                    "editing".bold().yellow(),
                    " mode, you can:".bold(),
                ]),
                Line::from(editing_keys),
                Line::from(""),
                Line::from(Span::raw("When choosing models, you can:").bold()),
                Line::from(model_keys),
                Line::from(""),
                Line::from(Span::raw("When choosing chats, you can:").bold()),
                Line::from(chat_keys),
                Line::from(""),
                Line::from(Span::raw("When browsing snippets, you can:").bold()),
                Line::from(snippet_keys),
                Line::from(""),
                Line::from(Span::raw("When exploring files, you can:").bold()),
                Line::from(file_explorer_keys),
                Line::from(""),
                Line::from(Span::raw("When viewing context:").bold()),
                Line::from(context_keys),
            ];
            let help_text_block = Block::new().padding(Padding::uniform(1));
            let text = Text::from(msg).patch_style(Style::default());
            let help_message = Paragraph::new(text)
                .scroll((app.help_scroll as u16, 0))
                .block(help_text_block)
                .wrap(Wrap { trim: true });
            f.render_widget(help_message, area);

            // Add scrollbar
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓"));
            let mut scrollbar_state = ScrollbarState::new(30).position(app.help_scroll);
            f.render_stateful_widget(
                scrollbar,
                area.inner(Margin {
                    vertical: 1,
                    horizontal: 0,
                }),
                &mut scrollbar_state,
            );
        }
        AppMode::ExploreFiles => {
            let area = centered_rect(80, 60, messages_area);
            render_popup(f, "Select File", area);
            render_file_explorer(f, area, app);
        }
        AppMode::ShowContext => {
            let area = centered_rect(40, 40, messages_area);
            render_popup(f, "Files Added to Context", area);
            render_context_list(f, area, app);
        }
        AppMode::Notify { notification } => {
            let area = centered_rect(40, 40, messages_area);
            render_popup(f, "Notification", area);
            render_notification(f, area, notification);
        }
    }

    let msg = match app.app_mode {
        AppMode::Editing => {
            vec![
                "Press ".into(),
                "Esc".bold(),
                " to stop editing. Press ".into(),
                "CONTROL + s (C-s)".bold(),
                " to submit the message.".into(),
            ]
        }
        AppMode::ExploreFiles => {
            vec![
                "Navigate: ".into(),
                "h/j/k/l or arrows".bold(),
                ". ".into(),
                "Enter".bold(),
                " to add file to context. ".into(),
                "d".bold(),
                " to remove from context. ".into(),
                "Esc/q".bold(),
                " to exit.".into(),
            ]
        }
        AppMode::ShowContext => {
            vec![
                "These files will be included in your next message. Press ".into(),
                "Esc/q/Enter".bold(),
                " to return.".into(),
            ]
        }
        AppMode::ModelSelection => {
            vec![
                "Navigate: ".into(),
                "j/k or Up/Down".bold(),
                ". ".into(),
                "Enter".bold(),
                " to select model. ".into(),
                "/".bold(),
                " to search. ".into(),
                "Esc/q".bold(),
                " to cancel.".into(),
            ]
        }
        AppMode::FilterModels | AppMode::FilterHistory => {
            vec![
                "Type to filter. ".into(),
                "Up/Down".bold(),
                " to navigate. ".into(),
                "Enter".bold(),
                " to select model. ".into(),
                "Esc".bold(),
                " to clear filter.".into(),
            ]
        }
        AppMode::ShowHistory => {
            vec![
                "Navigate: ".into(),
                "j/k or Up/Down".bold(),
                ". ".into(),
                "Enter".bold(),
                " to select chat. ".into(),
                "/".bold(),
                " to search. ".into(),
                "CONTROL + r (C-r)".bold(),
                " to delete chat. ".into(),
                "Esc/q".bold(),
                " to cancel.".into(),
            ]
        }
        AppMode::SnippetSelection => {
            vec![
                "Navigate: ".into(),
                "j/k or Up/Down".bold(),
                ". Press ".into(),
                "Enter/y".bold(),
                " to copy snippet. ".into(),
                "Esc/q".bold(),
                " to cancel.".into(),
            ]
        }
        _ => {
            vec![
                "Press ".into(),
                "Esc/q".bold(),
                " to exit. Press ".into(),
                "i".bold(),
                " to enter text. Press ".into(),
                "?".bold(),
                " for help.".into(),
            ]
        }
    };
    let text = Text::from(Line::from(msg)).patch_style(Style::default());
    let help_message = Paragraph::new(text);
    f.render_widget(help_message, help_area);

    #[cfg(not(target_os = "linux"))]
    {
        if let Some(cells) = app.selection.iter_selected_cells() {
            for (col, row) in cells {
                let cell = f.buffer_mut().cell_mut((col, row));
                // Modify the cell style to show selection
                if let Some(cell) = cell {
                    cell.set_style(SELECTED_STYLE);
                }
            }
        }

        if let Some(selected_text) = app.selection.get_selected_text(f.buffer_mut()) {
            // Trim whitespace from the selected text for each line
            let selected_text: String = selected_text
                .lines()
                .map(str::trim_end)
                .collect::<Vec<&str>>()
                .join("\n");
            app.clipboard.set_text(&selected_text).unwrap();
        }
    }
}

fn styled_list<'a>(items: Vec<ListItem<'a>>, block: Block<'a>) -> List<'a> {
    List::new(items)
        .block(block)
        .highlight_style(SELECTED_STYLE)
        .highlight_symbol(">")
        .highlight_spacing(HighlightSpacing::Always)
}

fn render_popup(f: &mut Frame, title: &str, area: Rect) {
    let block = Block::bordered().title(title);
    f.render_widget(Clear, area);
    f.render_widget(block, area);
}

fn render_model_list(f: &mut Frame, area: Rect, app: &mut App) {
    let block = Block::new().padding(Padding::uniform(1));
    if app.model_list.items.is_empty() {
        let p = Paragraph::new(
            Text::from("No API keys detected, no running Ollama detected. Unable to choose model.")
                .red(),
        )
        .wrap(Wrap { trim: true })
        .block(block);
        f.render_widget(p, area);
        return;
    }
    let indices = app.filtered_model_indices();
    let items: Vec<ListItem> = indices
        .iter()
        .map(|&i| ListItem::from(&app.model_list.items[i]))
        .collect();
    f.render_stateful_widget(styled_list(items, block), area, &mut app.model_list.state);
}

fn render_thinking_effort_list(f: &mut Frame, area: Rect, app: &mut App) {
    let block = Block::new().padding(Padding::uniform(1));
    let items: Vec<ListItem> = THINKING_EFFORTS
        .iter()
        .map(|&name| ListItem::from(name))
        .collect();
    f.render_stateful_widget(
        styled_list(items, block),
        area,
        &mut app.thinking_effort_state,
    );
}

fn render_snippet_list(f: &mut Frame, area: Rect, app: &mut App) {
    let block = Block::new().padding(Padding::uniform(1));
    let items: Vec<ListItem> = app
        .snippet_list
        .items
        .iter()
        .enumerate()
        .map(|(i, s)| {
            // Collect up to 11 chars to see if we need an ellipsis
            let chars: Vec<char> = s.text.chars().take(11).collect();
            let display_text = if chars.len() > 10 {
                // If it's longer than 10, take 10 and add "..."
                let truncated: String = chars.into_iter().take(10).collect();
                format!("{}...", truncated)
            } else {
                // Otherwise, just use the text as is
                chars.into_iter().collect()
            };
            ListItem::from(format!("Snippet {}: {}", i + 1, display_text))
        })
        .collect();
    f.render_stateful_widget(styled_list(items, block), area, &mut app.snippet_list.state);
}

fn render_chat_history_list(f: &mut Frame, area: Rect, app: &mut App) {
    let block = Block::new().padding(Padding::uniform(1));
    let items: Vec<ListItem> = app
        .chat_list
        .items
        .iter()
        .map(|c| ListItem::from(format!("Chat created {}", c.started_at)))
        .collect();
    f.render_stateful_widget(styled_list(items, block), area, &mut app.chat_list.state);
}

fn render_chat_history_panel(f: &mut Frame, messages_area: Rect, app: &mut App) {
    let area = left_aligned_rect(messages_area, 25);
    render_popup(f, "Select Chat", area);
    render_chat_history_list(f, area, app);

    let preview_area = right_aligned_rect(messages_area, 75);
    render_popup(f, "Chat Preview", preview_area);

    let preview_text = app.get_selected_chat_id().map(|id| {
        list_all_messages(*id)
            .unwrap_or_default()
            .into_iter()
            .map(|m| match m {
                Message::User(_) => format!("USER: {}\n", m),
                Message::Assistant(t) => format!("ASSISTANT: {t}\n"),
            })
            .collect::<Vec<_>>()
            .join("\n")
    });
    if let Some(text) = preview_text {
        f.render_widget(
            Paragraph::new(Text::from(text.as_str()).magenta())
                .wrap(Wrap { trim: true })
                .block(Block::new().padding(Padding::uniform(1))),
            preview_area,
        );
    }
}

fn render_file_explorer(f: &mut Frame, area: Rect, app: &mut App) {
    let layout = Layout::horizontal([Constraint::Ratio(1, 3), Constraint::Ratio(2, 3)]);
    let file_content = get_file_content(app.file_explorer.current());

    let file_content = match file_content {
        Ok(file_content) => file_content,
        _ => "Couldn't load file.".into(),
    };

    let chunks = layout.split(area);

    f.render_widget_ref(app.file_explorer.widget(), chunks[0]);
    f.render_widget(Clear, chunks[1]);
    f.render_widget(
        Paragraph::new(file_content).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Double),
        ),
        chunks[1],
    );
}

fn get_color(count: usize) -> Color {
    if count < 10000 {
        Color::Green
    } else if count < 50000 {
        Color::Yellow
    } else {
        Color::Red
    }
}

fn render_context_list(f: &mut Frame, area: Rect, app: &mut App) {
    if let Some(context) = &app.current_context {
        let text_block = Block::new().padding(Padding::uniform(1));

        let current_dir = env::current_dir().ok();

        let mut msg: Vec<Line<'_>> = context
            .iter()
            .map(|item| {
                let path = current_dir
                    .as_ref()
                    .and_then(|base| diff_paths(&item.file.path, base))
                    .unwrap_or_else(|| item.file.path.clone());

                let (tok_str, tok_color) = if let Some(count) = item.est_tokens {
                    (format!("{count}"), get_color(count))
                } else {
                    ("N/A".to_string(), Color::DarkGray)
                };

                Line::from(vec![
                    Span::raw(format!("File: {}, Est. tokens: ", path.to_string_lossy())),
                    Span::styled(tok_str, Style::default().fg(tok_color)),
                ])
            })
            .collect();

        let total_tokens: usize = context.iter().filter_map(|item| item.est_tokens).sum();

        msg.push(Line::raw("")); // Blank line for visual spacing

        msg.push(Line::from(vec![
            Span::styled(
                "Total Est. tokens: ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{total_tokens}"),
                Style::default()
                    .fg(get_color(total_tokens))
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        let text = Text::from(msg).patch_style(Style::default());
        let context_text = Paragraph::new(text)
            .block(text_block)
            .wrap(Wrap { trim: true });

        f.render_widget(context_text, area);
    };
}

fn render_notification(f: &mut Frame, area: Rect, notification: &Notification) {
    let text_block = Block::new().padding(Padding::uniform(1));
    let text = match notification {
        Notification::Info(message) => Text::from(message.clone()).patch_style(Style::default()),
        Notification::Error(message) => {
            Text::from(message.clone()).patch_style(Style::default().fg(Color::Red))
        }
        Notification::TokenEstimate(info) => match info {
            (Some(count), info_text) => {
                let (tok_str, tok_color) = (format!("{count}"), get_color(*count));
                Text::from(vec![
                    Line::raw(info_text),
                    Line::from(vec![
                        Span::raw("Est. token usage: "),
                        Span::styled(tok_str, Style::default().fg(tok_color)),
                    ]),
                ])
            }
            (None, info_text) => Text::from(vec![
                Line::raw(info_text),
                Line::styled(
                    "Could not estimate token usage.",
                    Style::default().fg(DarkGray),
                ),
            ]),
        },
    };
    let context_text = Paragraph::new(text)
        .block(text_block)
        .wrap(Wrap { trim: true });
    f.render_widget(context_text, area);
}
