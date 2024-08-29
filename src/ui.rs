use std::cmp::min;

use ratatui::{
    layout::{Alignment, Constraint, Layout, Margin, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Clear, HighlightSpacing, List, ListItem, Padding, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Wrap,
    },
    Frame,
};

use crate::app::{App, AppMode};

pub const SELECTED_STYLE: Style = Style::new().add_modifier(Modifier::BOLD).fg(Color::Green);

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

fn right_aligned_rect(r: Rect) -> Rect {
    Layout::horizontal([Constraint::Percentage(60), Constraint::Fill(1)]).split(r)[1]
}

pub fn render(f: &mut Frame, app: &mut App) {
    f.render_widget(
        Block::bordered()
            .title("AI in the Terminal")
            .title_alignment(Alignment::Center)
            .border_type(BorderType::Rounded),
        f.area(),
    );
    let input_area_constraint = match app.app_mode {
        AppMode::Editing => Constraint::Min(1),
        _ => Constraint::Length(0),
    };

    let vertical = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        input_area_constraint,
    ]);

    let vertical = vertical.margin(1);

    let [help_area, messages_area, input_area] = vertical.areas(f.area());

    let (msg, style) = match app.app_mode {
        AppMode::Editing => (
            vec![
                "Press ".into(),
                "Esc".bold(),
                " to stop editing. Press ".into(),
                "Enter + ALT".bold(),
                " to submit the message.".into(),
            ],
            Style::default(),
        ),
        _ => (
            vec![
                "Press ".into(),
                "Esc/q".bold(),
                " to exit. Press ".into(),
                "?".bold(),
                " for help.".into(),
            ],
            Style::default(),
        ),
    };
    let text = Text::from(Line::from(msg)).patch_style(style);
    let help_message = Paragraph::new(text);
    f.render_widget(help_message, help_area);

    if let AppMode::Editing = app.app_mode {
        f.render_widget(&app.input_textarea, input_area);
    }
    let messages: Vec<Line> = app
        .messages
        .iter()
        .enumerate()
        .flat_map(|(i, m)| {
            let wrapped_message = textwrap::wrap(m, messages_area.width as usize - 3);
            let mut line_vec = Vec::new();
            if i % 2 == 0 {
                line_vec.push(Line::from(Span::raw("USER:").bold().yellow()));
                line_vec.push(Line::from(Span::raw("---").bold().yellow()));
                line_vec.extend(
                    wrapped_message
                        .into_iter()
                        .map(|l| Line::from(Span::raw(l).yellow())),
                );
                line_vec.push(Line::from(Span::raw("").bold().yellow()));
            } else if m.starts_with("Error:") {
                line_vec.push(Line::from(Span::raw("ERROR:").bold().red()));
                line_vec.push(Line::from(Span::raw("---").bold().red()));
                line_vec.extend(
                    wrapped_message
                        .into_iter()
                        .map(|l| Line::from(Span::raw(l).red())),
                );
                line_vec.push(Line::from(Span::raw("").bold().red()));
            } else {
                line_vec.push(Line::from(Span::raw("ASSISTANT:").bold().green()));
                line_vec.push(Line::from(Span::raw("---").bold().green()));
                line_vec.extend(
                    wrapped_message
                        .into_iter()
                        .map(|l| Line::from(Span::raw(l).green())),
                );
                line_vec.push(Line::from(Span::raw("").bold().green()));
            }
            line_vec
        })
        .collect();

    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("↑"))
        .end_symbol(Some("↓"));

    let mut scrollbar_state = ScrollbarState::new(messages.len()).position(app.vertical_scroll);

    let messages_text = Text::from(messages);
    let messages = Paragraph::new(messages_text)
        .scroll((app.vertical_scroll as u16, 0))
        .block(Block::bordered().title(format!("Chat - {}", app.selected_model_name)));

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
    if let AppMode::ModelSelection = app.app_mode {
        let block = Block::bordered().title("Select Model");
        let area = centered_rect(40, 50, messages_area);
        f.render_widget(Clear, area); //this clears out the background
        f.render_widget(block, area);
        render_model_list(f, area, app);
    }

    if let AppMode::SnippetSelection = app.app_mode {
        let block = Block::bordered().title("Select Snippet");
        let area = centered_rect(20, 100, messages_area);
        f.render_widget(Clear, area); //this clears out the background
        f.render_widget(block, area);
        render_snippet_list(f, area, app);

        let preview_block = Block::bordered().title("Snippet Preview");
        let preview_area = right_aligned_rect(messages_area);
        f.render_widget(Clear, preview_area); //this clears out the background
        f.render_widget(preview_block, preview_area);
        let preview_text = app.get_snippet_text();
        let preview_block_content = Block::new().padding(Padding::uniform(1));
        if let Some(preview_text) = preview_text {
            let snippet_paragraph =
                Paragraph::new(Text::from(preview_text).magenta()).block(preview_block_content);
            f.render_widget(snippet_paragraph, preview_area);
        }
    }
    if let AppMode::Help = app.app_mode {
        let block = Block::bordered().title("Help");
        let area = centered_rect(50, 60, messages_area);
        f.render_widget(Clear, area); //this clears out the background
        f.render_widget(block, area);

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
            "s".bold(),
            " to browse code snippets.".into(),
        ];
        let editing_keys = vec![
            "Press ".into(),
            "Esc".bold(),
            " to stop editing. Press ".into(),
            "Enter + ALT".bold(),
            " to submit the message. ".into(),
            "Paste into the text area by pressing ".into(),
            "Ctrl + V".bold(),
        ];
        let model_keys = vec![
            "Press ".into(),
            "Up/Down".bold(),
            " to select model, or press ".into(),
            "Enter".bold(),
            " to select model, and return to 'normal' mode.".into(),
        ];
        let snippet_keys = vec![
            "Press ".into(),
            "Up/Down".bold(),
            " to select snippet, or press ".into(),
            "Enter".bold(),
            " to copy snippet to the clipboard (not linux yet), and return to 'normal' mode."
                .into(),
        ];
        let msg = vec![
            Line::from(Span::raw("Welcome to AI in the Terminal! ").bold()),
            Line::from(""),
            Line::from(Span::raw("When in 'normal' mode, you can:").bold()),
            Line::from(normal_keys),
            Line::from(""),
            Line::from(Span::raw("When in 'editing' mode, you can:").bold()),
            Line::from(editing_keys),
            Line::from(""),
            Line::from(Span::raw("When choosing models, you can:").bold()),
            Line::from(model_keys),
            Line::from(""),
            Line::from(Span::raw("When browsing snippets, you can:").bold()),
            Line::from(snippet_keys),
        ];
        let help_text_block = Block::new().padding(Padding::uniform(1));
        let text = Text::from(msg).patch_style(style);
        let help_message = Paragraph::new(text)
            .block(help_text_block)
            .wrap(Wrap { trim: true });
        f.render_widget(help_message, area);
    }
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
    // Iterate through all elements in the `items` and stylize them.
    let items: Vec<ListItem> = app.model_list.items.iter().map(ListItem::from).collect();

    // Create a List from all list items and highlight the currently selected one
    let list = List::new(items)
        .block(block)
        .highlight_style(SELECTED_STYLE)
        .highlight_symbol(">")
        .highlight_spacing(HighlightSpacing::Always);

    // We need to disambiguate this trait method as both `Widget` and `StatefulWidget` share the
    // same method name `render`.
    f.render_stateful_widget(list, area, &mut app.model_list.state);
}

fn render_snippet_list(f: &mut Frame, area: Rect, app: &mut App) {
    let block = Block::new().padding(Padding::uniform(1));

    // Iterate through all elements in the `items` and stylize them.
    let items: Vec<ListItem> = app
        .snippet_list
        .items
        .iter()
        .enumerate()
        .map(|(i, s)| {
            ListItem::from(format!(
                "Snippet {}: {}...",
                i + 1,
                s.text[..min(10, s.text.len())].to_owned()
            ))
        })
        .collect();

    // Create a List from all list items and highlight the currently selected one
    let list = List::new(items)
        .block(block)
        .highlight_style(SELECTED_STYLE)
        .highlight_symbol(">")
        .highlight_spacing(HighlightSpacing::Always);

    // We need to disambiguate this trait method as both `Widget` and `StatefulWidget` share the
    // same method name `render`.
    f.render_stateful_widget(list, area, &mut app.snippet_list.state);
}
