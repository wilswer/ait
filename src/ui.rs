use std::cmp::min;

use ratatui::{
    layout::{Alignment, Constraint, Flex, Layout, Margin, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Clear, HighlightSpacing, List, ListItem, Padding, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Wrap,
    },
    Frame,
};
use tui_big_text::{BigText, PixelSize};

use crate::{
    app::{App, AppMode, Message},
    storage::list_all_messages,
};

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

fn right_aligned_rect(r: Rect, p: u16) -> Rect {
    Layout::horizontal([Constraint::Percentage(100 - p), Constraint::Fill(1)]).split(r)[1]
}

fn left_aligned_rect(r: Rect, p: u16) -> Rect {
    Layout::horizontal([Constraint::Fill(1), Constraint::Percentage(100 - p)]).split(r)[0]
}

fn render_messages(f: &mut Frame, app: &mut App, messages_area: Rect) {
    let messages: Vec<Line> = app
        .messages
        .iter()
        .flat_map(|m| {
            let wrapped_message = textwrap::wrap(m.as_ref(), messages_area.width as usize - 3);
            let mut line_vec = Vec::new();
            match m {
                Message::User(_) => {
                    line_vec.push(Line::from(Span::raw("USER:").bold().yellow()));
                    line_vec.push(Line::from(Span::raw("---").bold().yellow()));
                    line_vec.extend(
                        wrapped_message
                            .into_iter()
                            .map(|l| Line::from(Span::raw(l).yellow())),
                    );
                    line_vec.push(Line::from(Span::raw("").bold().yellow()));
                }
                Message::Assistant(_) => {
                    line_vec.push(Line::from(Span::raw("ASSISTANT:").bold().green()));
                    line_vec.push(Line::from(Span::raw("---").bold().green()));
                    line_vec.extend(
                        wrapped_message
                            .into_iter()
                            .map(|l| Line::from(Span::raw(l).green())),
                    );
                    line_vec.push(Line::from(Span::raw("").bold().green()));
                }
                Message::Error(_) => {
                    line_vec.push(Line::from(Span::raw("ERROR:").bold().red()));
                    line_vec.push(Line::from(Span::raw("---").bold().red()));
                    line_vec.extend(
                        wrapped_message
                            .into_iter()
                            .map(|l| Line::from(Span::raw(l).red())),
                    );
                    line_vec.push(Line::from(Span::raw("").bold().red()));
                }
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
    // let text = Text::raw("Hi there");
    let centered_area = center_rect(area, Constraint::Length(26), Constraint::Length(8)); // 3 8x8
                                                                                          // characters
    f.render_widget(big_text, centered_area);
}
pub fn render(f: &mut Frame, app: &mut App) {
    let title = format!("AI in the Terminal (AIT v{})", env!("CARGO_PKG_VERSION"));
    let main_block = Block::bordered()
        .title(title)
        .title_alignment(Alignment::Center)
        .border_type(BorderType::Rounded);
    match app.app_mode {
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

    let vertical = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        input_area_constraint,
    ]);

    let vertical = vertical.margin(1);

    let [help_area, messages_area, input_area] = vertical.areas(f.area());

    match app.app_mode {
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
            let block = Block::bordered().title("Select Model");
            let area = centered_rect(40, 50, messages_area);
            f.render_widget(Clear, area); //this clears out the background
            f.render_widget(block, area);
            render_model_list(f, area, app);
        }
        AppMode::SnippetSelection => {
            let block = Block::bordered().title("Select Snippet");
            let area = left_aligned_rect(messages_area, 25);
            f.render_widget(Clear, area); //this clears out the background
            f.render_widget(block, area);
            render_snippet_list(f, area, app);

            let preview_block = Block::bordered().title("Snippet Preview");
            let preview_area = right_aligned_rect(messages_area, 75);
            f.render_widget(Clear, preview_area); //this clears out the background
            f.render_widget(preview_block, preview_area);
            let preview_text = app.get_snippet_text();
            let preview_block_content = Block::new().padding(Padding::uniform(1));
            if let Some(preview_text) = preview_text {
                let snippet_paragraph = Paragraph::new(Text::from(preview_text.as_str()).magenta())
                    .block(preview_block_content);
                f.render_widget(snippet_paragraph, preview_area);
            }
        }
        AppMode::ShowHistory => {
            let block = Block::bordered().title("Select Chat");
            let area = left_aligned_rect(messages_area, 25);
            f.render_widget(Clear, area); //this clears out the background
            f.render_widget(block, area);
            render_chat_history_list(f, area, app);

            let preview_block = Block::bordered().title("Chat Preview");
            let preview_area = right_aligned_rect(messages_area, 75);
            f.render_widget(Clear, preview_area); //this clears out the background
            f.render_widget(preview_block, preview_area);
            let chat_id = app.get_selected_chat_id();
            let preview_text = if let Some(id) = chat_id {
                let text = list_all_messages(*id)
                    .unwrap_or([].to_vec())
                    .into_iter()
                    .map(|m| match m {
                        Message::User(t) => format!("USER: {}\n", t),
                        Message::Assistant(t) => format!("ASSISTANT: {}\n", t),
                        Message::Error(t) => format!("ERROR: {}\n", t),
                    })
                    .collect::<Vec<String>>()
                    .join("\n");
                Some(text)
            } else {
                None
            };
            let preview_block_content = Block::new().padding(Padding::uniform(1));
            if let Some(preview_text) = preview_text {
                let snippet_paragraph = Paragraph::new(Text::from(preview_text.as_str()).magenta())
                    .wrap(Wrap { trim: true })
                    .block(preview_block_content);
                f.render_widget(snippet_paragraph, preview_area);
            }
        }
        AppMode::Help => {
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
                "h".bold(),
                " to browse previous conversations, ".into(),
                "s".bold(),
                " to browse code snippets.".into(),
            ];
            let editing_keys = vec![
                "Press ".into(),
                "Esc".bold(),
                " to stop editing. Press ".into(),
                "CONTROL + S (C-s)".bold(),
                " to submit the message. ".into(),
                "Paste into the text area by pressing ".into(),
                "Ctrl + V".bold(),
            ];
            let model_keys = vec![
                "Press ".into(),
                "Up/Down".bold(),
                " to select model, or press ".into(),
                "Enter".bold(),
                " to select model, which immediately enters 'editing' mode.".into(),
            ];
            let chat_keys = vec![
                "Press ".into(),
                "Up/Down".bold(),
                " to select chat, or press ".into(),
                "d".bold(),
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
                Line::from(Span::raw("When choosing chats, you can:").bold()),
                Line::from(chat_keys),
                Line::from(""),
                Line::from(Span::raw("When browsing snippets, you can:").bold()),
                Line::from(snippet_keys),
            ];
            let help_text_block = Block::new().padding(Padding::uniform(1));
            let text = Text::from(msg).patch_style(Style::default());
            let help_message = Paragraph::new(text)
                .block(help_text_block)
                .wrap(Wrap { trim: true });
            f.render_widget(help_message, area);
        }
    }

    let msg = match app.app_mode {
        AppMode::Editing => {
            vec![
                "Press ".into(),
                "Esc".bold(),
                " to stop editing. Press ".into(),
                "CONTROL + S (C-s)".bold(),
                " to submit the message.".into(),
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

fn render_chat_history_list(f: &mut Frame, area: Rect, app: &mut App) {
    let block = Block::new().padding(Padding::uniform(1));

    // Iterate through all elements in the `items` and stylize them.
    let items: Vec<ListItem> = app
        .chat_list
        .items
        .iter()
        .map(|c| ListItem::from(format!("Chat created {}", c.started_at)))
        .collect();

    // Create a List from all list items and highlight the currently selected one
    let list = List::new(items)
        .block(block)
        .highlight_style(SELECTED_STYLE)
        .highlight_symbol(">")
        .highlight_spacing(HighlightSpacing::Always);

    // We need to disambiguate this trait method as both `Widget` and `StatefulWidget` share the
    // same method name `render`.
    f.render_stateful_widget(list, area, &mut app.chat_list.state);
}
