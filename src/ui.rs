use std::borrow::Cow;
use std::env;

use genai::ModelSpec;
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
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    app::{App, AppMode, Message, Notification, THINKING_EFFORTS, get_file_content},
    snippets::{
        MessageSegment, MessageText, create_highlighted_code, parse_message_segments,
        translate_language_name_to_syntect_name,
    },
    storage::list_all_messages,
};

const AIT_ASCII: &str = include_str!("../assets/ait.txt");

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

fn right_aligned_rect_percent(r: Rect, p: u16) -> Rect {
    Layout::horizontal([Constraint::Percentage(100 - p), Constraint::Fill(1)]).split(r)[1]
}

fn left_aligned_rect_percent(r: Rect, p: u16) -> Rect {
    Layout::horizontal([Constraint::Fill(1), Constraint::Percentage(100 - p)]).split(r)[0]
}

fn make_rects_from_left_aligned_constraint(r: Rect, l: u16) -> (Rect, Rect) {
    let rects = Layout::horizontal([Constraint::Length(l), Constraint::Fill(1)]).split(r);
    (rects[0], rects[1])
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
    let raw_lines: Vec<&str> = text.lines().collect();
    let mut i = 0;

    while i < raw_lines.len() {
        let line = raw_lines[i];
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();

        // Check if this line starts a table block (starts and ends with '|')
        let looks_like_table = trimmed.starts_with('|') && trimmed.ends_with('|');
        if looks_like_table {
            // Collect all consecutive table rows
            let mut table_rows: Vec<&str> = Vec::new();
            while i < raw_lines.len() {
                let tr = raw_lines[i].trim();
                if !(tr.starts_with('|') && tr.ends_with('|')) {
                    break;
                }
                table_rows.push(raw_lines[i]);
                i += 1;
            }
            // Render the table block
            let table_lines = render_table_block(&table_rows, width, style);
            lines.extend(table_lines);
            continue; // i already advanced
        }

        if trimmed.is_empty() {
            lines.push(Line::default());
            i += 1;
            continue;
        }

        // Horizontal rule
        if is_separator(trimmed) {
            lines.push(
                Line::from("─".repeat(3)).style(style.patch(Style::default().fg(Color::DarkGray))),
            );
            i += 1;
            continue;
        }

        // ATX headings
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
            i += 1;
            continue;
        }

        // Unordered list
        let is_unordered =
            trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ");
        if is_unordered {
            let item_text = &trimmed[2..];
            let bullet_prefix = format!("{}• ", " ".repeat(indent));
            let prefix_w = bullet_prefix.chars().count();
            let avail = width.saturating_sub(prefix_w).max(1);

            // Parse inline markdown on the *whole* item text first so that
            // `**bold**` / `*italic*` / `` `code` `` pairs are matched against
            // the unbroken source — then wrap the already-styled spans.
            let item_spans = parse_inline_markdown(item_text, style);
            for (idx, line) in wrap_spans(&item_spans, avail).into_iter().enumerate() {
                let mut spans = if idx == 0 {
                    vec![Span::styled(
                        bullet_prefix.clone(),
                        style.patch(Style::default().fg(Color::DarkGray)),
                    )]
                } else {
                    vec![Span::styled(" ".repeat(prefix_w), style)]
                };
                spans.extend(line.spans);
                lines.push(Line::from(spans));
            }
            i += 1;
            continue;
        }

        // Ordered list
        let num_end = trimmed.find(". ").unwrap_or(0);
        let is_ordered = num_end > 0 && trimmed[..num_end].chars().all(|c| c.is_ascii_digit());
        if is_ordered {
            let num_prefix = format!("{}{}. ", " ".repeat(indent), &trimmed[..num_end]);
            let prefix_w = num_prefix.chars().count();
            let item_text = &trimmed[num_end + 2..];
            let avail = width.saturating_sub(prefix_w).max(1);

            let item_spans = parse_inline_markdown(item_text, style);
            for (idx, line) in wrap_spans(&item_spans, avail).into_iter().enumerate() {
                let mut spans = if idx == 0 {
                    vec![Span::styled(
                        num_prefix.clone(),
                        style.patch(Style::default().fg(Color::DarkGray)),
                    )]
                } else {
                    vec![Span::styled(" ".repeat(prefix_w), style)]
                };
                spans.extend(line.spans);
                lines.push(Line::from(spans));
            }
            i += 1;
            continue;
        }

        // Regular paragraph
        let full_spans = parse_inline_markdown(line, style);
        lines.extend(wrap_spans(&full_spans, width.max(1)));
        i += 1;
    }

    lines
}

/// Strip inline markdown markers (`**`, `*`, `` ` ``) to get the plain
/// display text, so we can measure the *visible* width of a cell.
pub fn strip_inline_markdown(text: &str) -> String {
    let mut result = String::new();
    let mut rest = text;
    while !rest.is_empty() {
        if rest.starts_with("**")
            && let Some(end) = rest[2..].find("**")
        {
            result.push_str(&rest[2..2 + end]);
            rest = &rest[2 + end + 2..];
            continue;
        }
        if rest.starts_with('*')
            && let Some(end) = rest[1..].find('*')
        {
            let inner = &rest[1..1 + end];
            if !inner.is_empty() && !inner.contains('\n') {
                result.push_str(inner);
                rest = &rest[1 + end + 1..];
                continue;
            }
        }
        if rest.starts_with('`')
            && let Some(end) = rest[1..].find('`')
        {
            let inner = &rest[1..1 + end];
            if !inner.is_empty() {
                result.push_str(inner);
                rest = &rest[1 + end + 1..];
                continue;
            }
        }
        let c = rest.chars().next().unwrap();
        result.push(c);
        rest = &rest[c.len_utf8()..];
    }
    result
}

/// Word-wrap a sequence of styled spans into multiple lines, preserving the
/// style of every (sub)span. Splits on whitespace; a single long word may
/// exceed `width`.
fn wrap_spans<'a>(spans: &[Span<'a>], width: usize) -> Vec<Line<'a>> {
    use unicode_width::UnicodeWidthStr;

    if width == 0 {
        return vec![Line::from(spans.to_vec())];
    }

    #[derive(Clone)]
    struct Tok<'a> {
        style: Style,
        text: std::borrow::Cow<'a, str>,
    }

    let mut tokens: Vec<Tok> = Vec::new();
    for span in spans {
        let style = span.style;
        let mut current_chunk = String::new();
        let mut current_is_space: Option<bool> = None;

        for c in span.content.chars() {
            let c_is_space = c == ' ';
            if current_is_space == Some(c_is_space) {
                current_chunk.push(c);
            } else {
                if !current_chunk.is_empty() {
                    tokens.push(Tok {
                        style,
                        text: std::mem::take(&mut current_chunk).into(),
                    });
                }
                current_chunk.push(c);
                current_is_space = Some(c_is_space);
            }
        }
        if !current_chunk.is_empty() {
            tokens.push(Tok {
                style,
                text: current_chunk.into(),
            });
        }
    }

    let mut out: Vec<Line<'a>> = Vec::new();
    let mut cur: Vec<Span<'a>> = Vec::new();
    let mut cur_w = 0usize;

    for tok in tokens {
        let w = UnicodeWidthStr::width(tok.text.as_ref());
        if cur_w + w > width && !cur.is_empty() {
            // drop trailing space from current line
            if cur.last().map(|s| s.content.as_ref()) == Some(" ") {
                cur.pop();
            }
            out.push(Line::from(std::mem::take(&mut cur)));
            cur_w = 0;
            // skip leading space of new line
            if tok.text == " " {
                continue;
            }
        }
        cur_w += w;
        cur.push(Span::styled(tok.text.into_owned(), tok.style));
    }
    if !cur.is_empty() {
        if cur.last().map(|s| s.content.as_ref()) == Some(" ") {
            cur.pop();
        }
        out.push(Line::from(cur));
    }
    if out.is_empty() {
        out.push(Line::default());
    }
    out
}

fn render_table_block(rows: &[&str], _width: usize, style: Style) -> Vec<Line<'static>> {
    if rows.len() < 2 {
        return Vec::new();
    }

    let style = style.patch(Style::default().bg(Color::Rgb(45, 45, 45)));

    fn parse_row(row: &str) -> Vec<String> {
        let r = row.trim();
        let inner = r.strip_prefix('|').unwrap_or(r);
        let inner = inner.strip_suffix('|').unwrap_or(inner);
        inner.split('|').map(|s| s.trim().to_string()).collect()
    }

    fn is_separator_row(cells: &[String]) -> bool {
        cells.iter().all(|c| {
            !c.is_empty()
                && c.chars().all(|ch| ch == '-' || ch == ':' || ch == ' ')
                && c.contains('-')
        })
    }

    fn alignment_from_cell(cell: &str) -> TableAlignment {
        let t = cell.trim();
        if t.starts_with(':') && t.ends_with(':') {
            TableAlignment::Center
        } else if t.ends_with(':') {
            TableAlignment::Right
        } else {
            TableAlignment::Left
        }
    }

    /// Display width of a cell, accounting for wide (CJK) characters and
    /// stripping markdown formatting markers.
    fn cell_display_width(text: &str) -> usize {
        UnicodeWidthStr::width(strip_inline_markdown(text).as_str())
    }

    /// Pad a row to have exactly `num_cols` cells (fill missing with empty strings).
    fn pad_row(row: &[String], num_cols: usize) -> Vec<String> {
        let mut r: Vec<String> = row.to_vec();
        while r.len() < num_cols {
            r.push(String::new());
        }
        r
    }

    let parsed_rows: Vec<Vec<String>> = rows.iter().map(|r| parse_row(r)).collect();
    if parsed_rows.len() < 2 {
        return Vec::new();
    }

    let header_cells = &parsed_rows[0];
    let separator_cells = &parsed_rows[1];

    let num_cols = header_cells.len();
    if num_cols == 0 {
        return Vec::new();
    }

    let (separator, data_rows) = if is_separator_row(separator_cells) {
        (Some(separator_cells), &parsed_rows[2..])
    } else {
        (None, &parsed_rows[1..])
    };

    let mut alignments = vec![TableAlignment::Left; num_cols];
    if let Some(sep) = separator {
        for (i, cell) in sep.iter().enumerate().take(num_cols) {
            alignments[i] = alignment_from_cell(cell);
        }
    }

    // Compute column widths based on *display* width (markdown stripped, wide chars counted)
    let mut col_widths = vec![0usize; num_cols];
    for (i, cell) in header_cells.iter().enumerate() {
        col_widths[i] = col_widths[i].max(cell_display_width(cell));
    }
    for row in data_rows {
        for (i, cell) in row.iter().enumerate().take(num_cols) {
            col_widths[i] = col_widths[i].max(cell_display_width(cell));
        }
    }

    let border_style = style.patch(Style::default().fg(Color::DarkGray).dim());
    let border_char = "│";

    let mut result = Vec::new();

    // Compute left/right padding for a cell given its alignment
    let cell_padding = |i: usize, raw_width: usize| -> (usize, usize) {
        let total_col_width = col_widths[i] + 2; // 1 space padding each side
        let spaces = total_col_width.saturating_sub(raw_width);
        match alignments[i] {
            TableAlignment::Left => (1, spaces.saturating_sub(1)),
            TableAlignment::Right => (spaces.saturating_sub(1), 1),
            TableAlignment::Center => (spaces / 2, spaces - spaces / 2),
        }
    };

    // --- Header row (honors alignment) ---
    {
        let header_cells = pad_row(header_cells, num_cols);
        let header_style = style.patch(Style::default().bold());
        let mut spans: Vec<Span> = vec![Span::styled(border_char, border_style)];
        for (i, cell_text) in header_cells.iter().enumerate() {
            let cell_style = header_style;
            let content_spans = parse_inline_markdown(cell_text, cell_style);
            let raw_width = cell_display_width(cell_text);
            let (left, right) = cell_padding(i, raw_width);
            spans.push(Span::styled(" ".repeat(left), cell_style));
            spans.extend(content_spans);
            spans.push(Span::styled(" ".repeat(right), cell_style));
            spans.push(Span::styled(border_char, border_style));
        }
        result.push(Line::from(spans));
    }

    // --- Separator row ---
    if let Some(sep) = separator {
        let sep = pad_row(sep, num_cols);
        let mut spans: Vec<Span> = vec![Span::styled(border_char, border_style)];
        for (i, cell) in sep.iter().enumerate().take(num_cols) {
            // Preserve alignment colons but generate dashes to exactly fill
            // col_widths[i], so the separator matches the data cell width.
            let t = cell.trim();
            let has_left = t.starts_with(':');
            let has_right = t.ends_with(':');
            let reserved = (has_left as usize) + (has_right as usize);
            let dash_n = col_widths[i].saturating_sub(reserved);
            let mut dashes = String::new();
            if has_left {
                dashes.push(':');
            }
            dashes.push_str(&"-".repeat(dash_n));
            if has_right {
                dashes.push(':');
            }
            let display = format!(" {} ", dashes);
            spans.push(Span::styled(display, border_style));
            spans.push(Span::styled(border_char, border_style));
        }
        result.push(Line::from(spans));
    }

    // --- Data rows ---
    for row in data_rows {
        let row = pad_row(row, num_cols);
        let mut spans: Vec<Span> = vec![Span::styled(border_char, border_style)];
        for (i, cell_text) in row.iter().enumerate().take(num_cols) {
            let cell_style = style;
            let content_spans = parse_inline_markdown(cell_text, cell_style);
            let raw_width = cell_display_width(cell_text);
            let (left, right) = cell_padding(i, raw_width);
            spans.push(Span::styled(" ".repeat(left), cell_style));
            spans.extend(content_spans);
            spans.push(Span::styled(" ".repeat(right), cell_style));
            spans.push(Span::styled(border_char, border_style));
        }
        result.push(Line::from(spans));
    }

    result
}

#[derive(Debug, Clone, Copy)]
enum TableAlignment {
    Left,
    Center,
    Right,
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
    title: Cow<'static, str>,
    align: BubbleAlign,
    border: Style,
}

fn user_skin() -> BubbleSkin {
    BubbleSkin {
        title: Cow::Borrowed("User"),
        align: BubbleAlign::Right,
        border: Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    }
}

fn assistant_skin() -> BubbleSkin {
    BubbleSkin {
        title: Cow::Borrowed("Assistant"),
        align: BubbleAlign::Left,
        border: Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    }
}

/// Build the title for an assistant bubble, including which model produced
/// the response. Falls back to "unknown" for messages that predate model
/// tracking (or while the model is not yet known).
fn assistant_title(model: Option<&str>, provider: Option<&str>) -> Cow<'static, str> {
    let model = model.unwrap_or("unknown");
    let provider = provider.unwrap_or("unknown");
    Cow::Owned(format!("Assistant ({model} -- {provider})"))
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

/// Build the styled spans for a chat-history-preview role header, dimming
/// and italicizing the model attribution `(<model> -- <provider>)` while the
/// role label and trailing colon use `role_style.bold()`.
fn style_preview_header(header: &str, role_style: Style) -> Vec<Span<'static>> {
    let label_style = role_style.bold();
    let attr_style = Style::default().dim().italic();
    match header.split_once('(') {
        Some((role, rest)) => {
            let mut spans = vec![Span::styled(role.to_string(), label_style)];
            spans.push(Span::styled("(".to_string(), attr_style));
            if let Some((attr, tail)) = rest.split_once(')') {
                spans.push(Span::styled(attr.to_string(), attr_style));
                spans.push(Span::styled(")".to_string(), attr_style));
                spans.push(Span::styled(tail.to_string(), label_style));
            } else {
                spans.push(Span::styled(rest.to_string(), attr_style));
            }
            spans
        }
        None => vec![Span::styled(header.to_string(), label_style)],
    }
}

/// Build the styled spans for an assistant bubble's top border header, with
/// the model attribution (`(<model> -- <provider>)`) dimmed and italicized
/// to de-emphasize it next to the bold "Assistant" role label. Only the
/// portion matching `attribution` (i.e. starting at the first `(`) is
/// de-emphasized; the role label and surrounding border chars use `border`.
fn bubble_title_spans(
    prefix: &str,
    title: &str,
    suffix: &str,
    border: Style,
) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let attr_style = Style::default().dim().italic();
    // Split the title into role + attribution at the first '('.
    match title.split_once('(') {
        Some((role, rest)) => {
            spans.push(Span::styled(prefix.to_string(), border));
            spans.push(Span::styled(role.to_string(), border));
            spans.push(Span::styled("(".to_string(), attr_style));
            // `rest` includes the trailing ')' (and any text after it).
            if let Some((attr, tail)) = rest.split_once(')') {
                spans.push(Span::styled(attr.to_string(), attr_style));
                spans.push(Span::styled(")".to_string(), attr_style));
                spans.push(Span::styled(tail.to_string(), border));
            } else {
                spans.push(Span::styled(rest.to_string(), attr_style));
            }
            spans.push(Span::styled(suffix.to_string(), border));
        }
        None => {
            // No attribution (e.g. plain "Assistant"): emit as a single span.
            spans.push(Span::styled(format!("{prefix}{title}{suffix}"), border));
        }
    }
    spans
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
        // Top border: ╭─ Assistant (<model> -- <provider>) ───────╮
        // The model attribution is separated out and dimmed/italicized so
        // the role label stays the visual anchor.
        let head_spans = bubble_title_spans("╭─ ", &skin.title, " ", skin.border);
        let head_w = skin.title.chars().count() + "╭─ ".chars().count() + 1;
        let fill = outer.saturating_sub(head_w + 1);
        let mut spans = head_spans;
        spans.push(Span::styled(format!("{}╮", "─".repeat(fill)), skin.border));
        lines.push(pad(spans));
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
        Message::Assistant(t, model, provider) => {
            if t.is_empty() {
                return Vec::new();
            }
            let mut skin = assistant_skin();
            skin.title = assistant_title(model.as_deref(), provider.as_deref());
            (skin, t.clone())
        }
    };
    let body = process_code_blocks(text, content_width, theme);
    let mut lines = frame_bubble(body, line_width, &skin);
    lines.push(Line::from(""));
    lines
}

/// Render an assistant "waiting for response" bubble with an animated spinner.
/// `model`/`provider` (taken from the in-flight stream state) are shown in
/// the bubble header so the user can see which model is responding.
fn waiting_bubble<'a>(
    line_width: usize,
    spinner_frame: usize,
    model: Option<&str>,
    provider: Option<&str>,
) -> Vec<Line<'a>> {
    let frame = SPINNER_FRAMES[(spinner_frame / 2) % SPINNER_FRAMES.len()];
    let thinking_split_n = (spinner_frame / 8) % THINKING_VERB.len();
    let (think1, think2) = THINKING_VERB.split_at(thinking_split_n);
    let body = vec![
        Line::from(vec![
            Span::raw(format!("{frame} ")),
            Span::raw(think1.to_string()).bold(),
            Span::raw(think2.to_string()).dim(),
        ])
        .style(Style::default().fg(Color::DarkGray)),
    ];
    let mut skin = assistant_skin();
    skin.title = assistant_title(model, provider);
    let mut lines = frame_bubble(body, line_width, &skin);
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
            Message::Assistant(m, model, provider) => {
                if m.is_empty() {
                    continue;
                }
                let mut skin = assistant_skin();
                skin.title = assistant_title(model.as_deref(), provider.as_deref());
                (skin, m.clone())
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
    let mut messages = if !app.is_view_streaming() && app.do_highlight {
        app.cached_lines.clone()
    } else {
        messages_to_lines(&app.messages, line_width)
    };

    if app.is_view_waiting() {
        // Pull the model/provider from the in-flight stream state so the
        // waiting bubble header shows which model is being queried.
        let (wm, wp) = match app.conversation_id.and_then(|id| app.streams.get(&id)) {
            Some(s) => crate::models::model_provider_from_spec(&s.selected_model),
            None => (None, None),
        };
        messages.extend(waiting_bubble(
            line_width,
            app.spinner_frame,
            wm.as_deref(),
            wp.as_deref(),
        ));
    }

    let mut scrollbar_state = ScrollbarState::new(messages.len()).position(app.vertical_scroll);

    let messages_text = Text::from(messages);
    let selected_model_info_str = match &app.selected_model {
        ModelSpec::Name(name) => name.as_str(),
        ModelSpec::Iden(iden) => &format!(
            "{}, provided by {}",
            iden.model_name.as_str(),
            iden.adapter_kind.as_str()
        ),
        ModelSpec::Target(target) => &format!(
            "{}, provided by {}",
            target.model.model_name.as_str(),
            target.model.adapter_kind.as_str(),
        ),
    };
    let messages = Paragraph::new(messages_text)
        .scroll((app.vertical_scroll as u16, 0))
        .block(Block::bordered().title(format!(
            "Chat - {} [effort: {}]",
            selected_model_info_str,
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

fn render_init_screen(f: &mut Frame, area: Rect, frame: usize) {
    let title = AIT_ASCII;
    let title_lines: Vec<&str> = title.lines().collect();
    let title_height = title_lines.len();
    let offset = 2;

    // --- Ripple animation ---
    let speed = 2; // spinner frames per line advance

    // Let the wave go slightly past the bottom so it fully leaves the screen
    let wave_len = title_height + offset + 5;
    let pause_len = title_height * 16;

    // Startup sequence: filling, wait
    let startup_len = wave_len + pause_len;

    // Loop sequence: emptying, filling, wait
    let loop_len = wave_len * 2 + pause_len;

    let step = frame / speed;

    let phase: u8;
    let wave_pos: usize;

    if step < startup_len {
        // --- Startup Sequence ---
        if step < wave_len {
            phase = 1; // Filling
            wave_pos = step;
        } else {
            phase = 4; // Wait
            wave_pos = 0;
        }
    } else {
        // --- Looping Sequence ---
        let loop_step = (step - startup_len) % loop_len;
        if loop_step < wave_len {
            phase = 2; // Emptying
            wave_pos = loop_step;
        } else if loop_step < wave_len * 2 {
            phase = 3; // Filling
            wave_pos = loop_step - wave_len;
        } else {
            phase = 4; // Wait
            wave_pos = 0;
        }
    }

    let styled_title: Vec<Line> = title_lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            if phase == 4 {
                return Line::from(*line).style(Style::default().bold().fg(Color::White));
            }

            let is_filling = phase == 1 || phase == 3;
            let dist = wave_pos as i32 - i as i32;

            let offset = offset as i32;
            let style = if is_filling {
                if dist >= offset + 5 {
                    Style::default().bold().fg(Color::White)
                } else if dist == offset + 4 {
                    Style::default().fg(Color::White)
                } else if dist == offset + 3 {
                    Style::default().fg(Color::Gray)
                } else if dist == offset + 2 {
                    Style::default().fg(Color::DarkGray)
                } else if dist == offset + 1 {
                    Style::default().dim().fg(Color::DarkGray)
                } else if dist == offset {
                    Style::default().fg(Color::Black)
                } else {
                    Style::default().dim().fg(Color::Black)
                }
            } else {
                if dist >= offset + 5 {
                    Style::default().dim().fg(Color::Black)
                } else if dist == offset + 4 {
                    Style::default().fg(Color::Black)
                } else if dist == offset + 3 {
                    Style::default().dim().fg(Color::DarkGray)
                } else if dist == offset + 2 {
                    Style::default().fg(Color::DarkGray)
                } else if dist == offset + 1 {
                    Style::default().fg(Color::Gray)
                } else if dist == offset {
                    Style::default().fg(Color::White)
                } else {
                    Style::default().bold().fg(Color::White)
                }
            };
            Line::from(*line).style(style)
        })
        .collect();

    // --- Build the instructions ---
    let key_style = Style::default().yellow().bold();
    let text_style = Style::default().fg(Color::Gray);

    let instructions = vec![
        Line::from(Span::styled(
            "AI in the Terminal",
            Style::default().fg(Color::Cyan).bold(),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("i", key_style),
            Span::styled("   Write a message", text_style),
        ]),
        Line::from(vec![
            Span::styled("m", key_style),
            Span::styled("   Choose a model", text_style),
        ]),
        Line::from(vec![
            Span::styled("h", key_style),
            Span::styled("   Browse chat history", text_style),
        ]),
        Line::from(vec![
            Span::styled("s", key_style),
            Span::styled("   Browse code snippets", text_style),
        ]),
        Line::from(vec![
            Span::styled("f", key_style),
            Span::styled("   Add files to context", text_style),
        ]),
        Line::from(vec![
            Span::styled("c", key_style),
            Span::styled("   View context files", text_style),
        ]),
        Line::from(vec![
            Span::styled("n", key_style),
            Span::styled("   Start a new chat", text_style),
        ]),
        Line::from(vec![
            Span::styled("t", key_style),
            Span::styled("   Change syntax theme", text_style),
        ]),
        Line::from(vec![
            Span::styled("?", key_style),
            Span::styled("   Full help", text_style),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Esc", key_style),
            Span::raw(" / "),
            Span::styled("q", key_style),
            Span::styled("  to exit", text_style),
        ]),
    ];

    let instructions_height = instructions.len() as u16;

    // Layout: flexible top, title, gap, instructions, flexible bottom
    let chunks = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(title_height as u16),
        Constraint::Length(2), // gap
        Constraint::Length(instructions_height),
        Constraint::Fill(1),
    ])
    .split(area);

    // Render the title
    let title_widget = Paragraph::new(Text::from(styled_title)).alignment(Alignment::Center);
    f.render_widget(title_widget, chunks[1]);

    // Measure the widest instruction line
    let instructions_width = instructions.iter().map(|l| l.width()).max().unwrap_or(0) as u16;

    // Center the instructions block horizontally under the title
    let [instructions_area] = Layout::horizontal([Constraint::Length(instructions_width)])
        .flex(Flex::Center)
        .areas(chunks[3]);

    // Render instructions (left-aligned within the centered area)
    let instructions_widget = Paragraph::new(Text::from(instructions)).alignment(Alignment::Left);
    f.render_widget(instructions_widget, instructions_area);
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
                render_init_screen(f, messages_area, app.spinner_frame);
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
            let area = left_aligned_rect_percent(messages_area, 25);
            render_popup(f, "Select Snippet", area);
            render_snippet_list(f, area, app);

            let preview_area = right_aligned_rect_percent(messages_area, 75);
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

    let mut msg = match app.app_mode {
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
    if app.active_stream_count() > 0 {
        msg.push("  ".into());
        msg.push(Span::styled(
            format!("⏳ streaming {} chat(s)", app.active_stream_count()),
            Style::default().fg(Color::Cyan),
        ));
        if app.is_view_streaming() || app.is_view_waiting() {
            msg.push(Span::styled(
                " (this chat) — u to cancel",
                Style::default().fg(Color::Yellow),
            ));
        }
    }
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

/// Case-insensitive substring test. Used to decide which lines of the chat
/// preview are shown while filtering conversations by message content.
fn ci_contains(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

/// If `slice` starts with a case-insensitive match of `needle` (compared
/// character by character), returns the byte length of the matched prefix.
/// Otherwise returns `None`. Unicode case folding is handled via
/// `char::to_lowercase`.
fn ci_match_len(slice: &str, needle: &[char]) -> Option<usize> {
    let mut si = slice.char_indices();
    for nc in needle {
        let (_, sc) = si.next()?;
        if !sc.to_lowercase().eq(nc.to_lowercase()) {
            return None;
        }
    }
    // Byte offset where the next char starts, or slice length if we consumed
    // everything — i.e. the byte length of the matched prefix.
    Some(match si.next() {
        Some((nb, _)) => nb,
        None => slice.len(),
    })
}

/// Split `line` into styled spans, wrapping every (case-insensitive)
/// occurrence of `query` in `match_style` and the surrounding text in
/// `base_style`. Slicing is byte-correct and Unicode-aware.
fn highlight_line(
    line: &str,
    query: &str,
    base_style: Style,
    match_style: Style,
) -> Vec<Span<'static>> {
    if query.is_empty() {
        return vec![Span::styled(line.to_string(), base_style)];
    }
    let needle: Vec<char> = query.chars().collect();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut rest = 0usize; // byte offset of the un-emitted prefix
    let mut pos = 0usize; // current search position
    let len = line.len();
    while pos < len {
        if let Some(mlen) = ci_match_len(&line[pos..], &needle) {
            let abs_end = pos + mlen;
            if pos > rest {
                spans.push(Span::styled(line[rest..pos].to_string(), base_style));
            }
            spans.push(Span::styled(line[pos..abs_end].to_string(), match_style));
            pos = abs_end;
            rest = pos;
        } else {
            // Advance one character boundary.
            match line[pos..].char_indices().nth(1) {
                Some((nb, _)) => pos += nb,
                None => break,
            }
        }
    }
    if rest < len {
        spans.push(Span::styled(line[rest..].to_string(), base_style));
    }
    if spans.is_empty() {
        spans.push(Span::styled(line.to_string(), base_style));
    }
    spans
}

fn render_chat_history_panel(f: &mut Frame, messages_area: Rect, app: &mut App) {
    let (area, preview_area) = make_rects_from_left_aligned_constraint(messages_area, 36);
    render_popup(f, "Select Chat", area);
    render_chat_history_list(f, area, app);

    render_popup(f, "Chat Preview", preview_area);

    // The active search query (if any) drives match highlighting and line
    // filtering in the preview. When a query is present, only lines that
    // contain it (case-insensitively) are shown, with every occurrence
    // highlighted in bold; otherwise the full conversation is shown plainly.
    let query: Option<String> = app
        .search_bar
        .lines()
        .first()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let preview_text = app.get_selected_chat_id().map(|id| {
        let messages = list_all_messages(*id).unwrap_or_default();
        let mut out: Vec<Line> = Vec::new();
        for m in messages {
            let (header, body, role_style): (Cow<'static, str>, String, Style) = match &m {
                Message::User(_) => (
                    Cow::Borrowed("USER:"),
                    m.to_string(),
                    Style::default().yellow(),
                ),
                Message::Assistant(t, model, provider) => {
                    let model = model.as_deref().unwrap_or("unknown");
                    let provider = provider.as_deref().unwrap_or("unknown");
                    (
                        Cow::Owned(format!("ASSISTANT: ({model} -- {provider})")),
                        t.clone(),
                        Style::default().green(),
                    )
                }
            };
            // Match style keeps the role color but adds bold + a highlight
            // background so occurrences are easy to spot in the preview.
            let match_style = role_style.add_modifier(Modifier::BOLD).bg(Color::DarkGray);

            let mut wrote_header = false;
            let mut last_emitted_idx: Option<usize> = None;
            // Trailing "\n" mirrors the original behaviour (a blank separator
            // line per message in the unfiltered view). Indexing the lines
            // lets us detect gaps between matches and insert a dim "..."
            // separator so the user can tell non-matching lines were skipped.
            let body_with_newline = format!("{body}\n");
            let lines: Vec<&str> = body_with_newline.split('\n').collect();
            for (i, line) in lines.iter().enumerate() {
                let matches = query.as_ref().is_some_and(|q| ci_contains(line, q));
                if query.is_some() && !matches {
                    continue;
                }
                if !wrote_header {
                    // De-emphasize the model attribution in the preview
                    // header to match the chat bubble styling: the role label
                    // and colon use the role color + bold, while the
                    // `(<model> -- <provider>)` part is dimmed/italicized.
                    let header_spans = style_preview_header(&header, role_style);
                    out.push(Line::from(header_spans));
                    wrote_header = true;
                }
                // Insert a "..." gap separator when lines were skipped between
                // the previously shown line and this one (also covers the gap
                // before the first match of a message). Only meaningful while
                // filtering — in the unfiltered view every line is shown.
                if query.is_some()
                    && match last_emitted_idx {
                        Some(prev) => prev + 1 != i,
                        None => i != 0,
                    }
                {
                    out.push(Line::from("...").style(Style::default().fg(Color::DarkGray).dim()));
                }
                let spans = match &query {
                    Some(q) => highlight_line(line, q, role_style, match_style),
                    None => vec![Span::styled(line.to_string(), role_style)],
                };
                out.push(Line::from(spans));
                last_emitted_idx = Some(i);
            }
        }
        out
    });
    if let Some(text) = preview_text {
        f.render_widget(
            Paragraph::new(text)
                .wrap(Wrap { trim: false })
                .block(Block::new().padding(Padding::uniform(1))),
            preview_area,
        );
    }
}

fn render_file_explorer(f: &mut Frame, area: Rect, app: &mut App) {
    let layout = Layout::horizontal([Constraint::Ratio(1, 3), Constraint::Ratio(2, 3)]);
    let file_content = get_file_content(&app.file_explorer.current().path);

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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;

    fn line_to_string(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn lines_to_strings(lines: &[Line]) -> Vec<String> {
        lines.iter().map(line_to_string).collect()
    }

    #[test]
    fn test_strip_inline_markdown() {
        assert_eq!(strip_inline_markdown("**bold**"), "bold");
        assert_eq!(strip_inline_markdown("*italic*"), "italic");
        assert_eq!(strip_inline_markdown("`code`"), "code");
        assert_eq!(
            strip_inline_markdown("Hello **world** and *universe*!"),
            "Hello world and universe!"
        );
    }

    #[test]
    fn test_simple_table() {
        let rows = vec![
            "| Header 1 | Header 2 |",
            "|----------|----------|",
            "| Cell 1   | Cell 2   |",
        ];
        let lines = render_table_block(&rows, 80, Style::default());
        let strings = lines_to_strings(&lines);

        assert_eq!(
            strings,
            vec![
                "│ Header 1 │ Header 2 │",
                "│ -------- │ -------- │",
                "│ Cell 1   │ Cell 2   │",
            ]
        );
    }

    #[test]
    fn test_table_with_alignments() {
        let rows = vec![
            "| Name | Age |",
            "|:-----|----:|",
            "| **Alice** | 30 |",
            "| Bob | 100 |",
        ];
        let lines = render_table_block(&rows, 80, Style::default());
        let strings = lines_to_strings(&lines);

        assert_eq!(
            strings,
            vec![
                "│ Name  │ Age │",
                "│ :---- │ --: │",
                "│ Alice │  30 │",
                "│ Bob   │ 100 │",
            ]
        );
    }

    #[test]
    fn test_ragged_table_rows() {
        let rows = vec![
            "| A | B | C |",
            "|---|---|---|",
            "| 1 |",             // missing cells
            "| 2 | 3 | 4 | 5 |", // extra cells
        ];
        let lines = render_table_block(&rows, 80, Style::default());
        let strings = lines_to_strings(&lines);

        assert_eq!(
            strings,
            vec![
                "│ A │ B │ C │",
                "│ - │ - │ - │",
                "│ 1 │   │   │", // padded with spaces
                "│ 2 │ 3 │ 4 │", // "5" is truncated
            ]
        );
    }

    #[test]
    fn test_wide_characters_and_markdown() {
        let rows = vec![
            "| Item | Count |",
            "|------|-------|",
            "| 日 本 語  | **5** |",
            "| English | 10 |",
        ];
        let lines = render_table_block(&rows, 80, Style::default());
        let strings = lines_to_strings(&lines);

        // "日 本 語 " is 3 chars, but 6 display columns wide.
        // "English" is 7 chars wide, so col 1 width = 7.
        assert_eq!(
            strings,
            vec![
                "│ Item     │ Count │",
                "│ -------- │ ----- │",
                "│ 日 本 語 │ 5     │",
                "│ English  │ 10    │",
            ]
        );
    }

    #[test]
    fn test_all_lines_same_width() {
        let markdown = r#"
| World | Atmosphere | Sky Color | Why? |
|-------|------------|-----------|------|
| **Mars** | Thin CO₂ + fine iron oxide dust | Butterscotch daytime, **blue sunset** | Dust scatters forward; fine particles scatter blue backward at low sun angles |
| **Venus** | Thick CO₂ + sulfuric acid clouds | Yellow-orange, hazy | Mie scattering + cloud absorption dominate |
| **Titan** | N₂ + methane + organic haze (tholins) | Orange/red | Complex hydrocarbon aerosols absorb blue/green |
| **Moon** | None | Pitch black (day or night) | No atmosphere = no scattering |
"#;
        let rows: Vec<&str> = markdown.trim().lines().collect();
        let lines = render_table_block(&rows, 200, Style::default());
        let strings = lines_to_strings(&lines);

        let first_width = UnicodeWidthStr::width(strings[0].as_str());
        assert!(first_width > 0, "Table should have width");

        for (i, s) in strings.iter().enumerate() {
            let w = UnicodeWidthStr::width(s.as_str());
            assert_eq!(
                w, first_width,
                "Line {} has width {} but expected {}. Content: {:?}",
                i, w, first_width, s
            );
        }
    }
}
