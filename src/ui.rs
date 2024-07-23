use ratatui::{
    layout::{Alignment, Constraint, Layout, Margin},
    style::{Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
    },
    Frame,
};

use crate::app::{App, InputMode};

pub fn render(f: &mut Frame, app: &App) {
    f.render_widget(
        Block::bordered()
            .title("Generative AI in the Terminal")
            .title_alignment(Alignment::Center)
            .border_type(BorderType::Rounded),
        f.size(),
    );

    let vertical = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Max(8),
    ]);

    let vertical = vertical.margin(1);

    let [help_area, messages_area, input_area] = vertical.areas(f.size());

    let (msg, style) = match app.input_mode {
        InputMode::Normal => (
            vec![
                "Press ".into(),
                "q".bold(),
                " to exit, ".into(),
                "i".bold(),
                " to start editing.".bold(),
                " Press Enter".bold(),
                " to submit the message.".into(),
            ],
            Style::default().add_modifier(Modifier::RAPID_BLINK),
        ),
        InputMode::Editing => (
            vec!["Press ".into(), "Esc".bold(), " to stop editing.".into()],
            Style::default(),
        ),
    };
    let text = Text::from(Line::from(msg)).patch_style(style);
    let help_message = Paragraph::new(text);
    f.render_widget(help_message, help_area);

    f.render_widget(app.text_area.widget(), input_area);

    let messages: Vec<Line> = app
        .messages
        .iter()
        .flat_map(|m| {
            if m.starts_with("USER:") {
                m.split('\n')
                    .map(|l| Line::from(Span::raw(l).yellow()))
                    .collect::<Vec<Line>>()
            } else {
                m.split('\n')
                    .map(|l| Line::from(Span::raw(l).green()))
                    .collect::<Vec<Line>>()
            }
        })
        .collect();

    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("↑"))
        .end_symbol(Some("↓"));

    let mut scrollbar_state = ScrollbarState::new(messages.len()).position(app.vertical_scroll);

    let messages_text = Text::from(messages);
    let messages = Paragraph::new(messages_text)
        .wrap(Wrap { trim: true })
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
}
