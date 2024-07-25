use ratatui::{
    layout::{Alignment, Constraint, Layout, Margin, Rect},
    style::{Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
    },
    Frame,
};

use crate::app::{App, AppMode};

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

pub fn render(f: &mut Frame, app: &App) {
    f.render_widget(
        Block::bordered()
            .title("Generative AI in the Terminal")
            .title_alignment(Alignment::Center)
            .border_type(BorderType::Rounded),
        f.size(),
    );
    let input_area_constraint = match app.app_mode {
        AppMode::Normal | AppMode::ModelSelection => Constraint::Length(0),
        AppMode::Editing => Constraint::Min(1),
    };

    let vertical = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        input_area_constraint,
    ]);

    let vertical = vertical.margin(1);

    let [help_area, messages_area, input_area] = vertical.areas(f.size());

    let (msg, style) = match app.app_mode {
        AppMode::Normal | AppMode::ModelSelection => (
            vec![
                "Press ".into(),
                "Esc/q".bold(),
                " to exit, ".into(),
                "i".bold(),
                " to start editing, ".into(),
                "y".bold(),
                " to copy the last answer, ".into(),
                "m".bold(),
                " to choose model. ".into(),
            ],
            Style::default().add_modifier(Modifier::RAPID_BLINK),
        ),
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
    };
    let text = Text::from(Line::from(msg)).patch_style(style);
    let help_message = Paragraph::new(text);
    f.render_widget(help_message, help_area);
    if let AppMode::Editing = app.app_mode {
        f.render_widget(app.input_textarea.widget(), input_area);
    }

    let messages: Vec<Line> = app
        .messages
        .iter()
        .enumerate()
        .flat_map(|(i, m)| {
            if i % 2 == 0 {
                let mut line_vec = Vec::new();
                line_vec.push(Line::from(Span::raw("USER:").bold().yellow()));
                line_vec.push(Line::from(Span::raw("---").bold().yellow()));
                line_vec.extend(m.split('\n').map(|l| Line::from(Span::raw(l).yellow())));
                line_vec.push(Line::from(Span::raw("").bold().yellow()));
                line_vec
            } else {
                let mut line_vec = Vec::new();
                line_vec.push(Line::from(Span::raw("ASSISTANT:").bold().green()));
                line_vec.push(Line::from(Span::raw("---").bold().green()));
                line_vec.extend(m.split('\n').map(|l| Line::from(Span::raw(l).green())));
                line_vec.push(Line::from(Span::raw("").bold().green()));
                line_vec
            }
        })
        .collect();

    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("↑"))
        .end_symbol(Some("↓"));

    let mut scrollbar_state = ScrollbarState::new(messages.len()).position(app.vertical_scroll);

    let messages_text = Text::from(messages);
    let messages = Paragraph::new(messages_text)
        .scroll((app.vertical_scroll as u16, 0))
        .block(Block::bordered().title("Chat"));

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
    }
}
