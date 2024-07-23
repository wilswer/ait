use ratatui::{
    layout::{Alignment, Constraint, Layout},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Paragraph, Wrap},
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
        Constraint::Length(3),
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
            ],
            Style::default().add_modifier(Modifier::RAPID_BLINK),
        ),
        InputMode::Editing => (
            vec![
                "Press ".into(),
                "Esc".bold(),
                " to stop editing, ".into(),
                "Enter".bold(),
                " to record the message".into(),
            ],
            Style::default(),
        ),
    };
    let text = Text::from(Line::from(msg)).patch_style(style);
    let help_message = Paragraph::new(text);
    f.render_widget(help_message, help_area);

    let input = Paragraph::new(app.input.as_str())
        .style(match app.input_mode {
            InputMode::Normal => Style::default(),
            InputMode::Editing => Style::default().fg(Color::Yellow),
        })
        .block(Block::bordered().title("Input"));
    f.render_widget(input, input_area);
    match app.input_mode {
        InputMode::Normal =>
            // Hide the cursor. `Frame` does this by default, so we don't need to do anything here
            {}

        InputMode::Editing => {
            // Make the cursor visible and ask ratatui to put it at the specified coordinates after
            // rendering
            #[allow(clippy::cast_possible_truncation)]
            f.set_cursor(
                // Draw the cursor at the current position in the input field.
                // This position is can be controlled via the left and right arrow key
                input_area.x + app.character_index as u16 + 1,
                // Move one line down, from the border to the input line
                input_area.y + 1,
            );
        }
    }

    let messages: Vec<Line> = app
        .user_messages
        .iter()
        .zip(app.bot_messages.iter())
        .flat_map(|(u, b)| {
            let mut user_lines = u
                .split('\n')
                .map(|l| Line::from(Span::raw(l).yellow()))
                .collect::<Vec<Line>>();
            user_lines.insert(0, Line::from(Span::raw("USER:").yellow()));
            let mut bot_lines = b
                .split('\n')
                .map(|l| Line::from(Span::raw(l).green()))
                .collect::<Vec<Line>>();
            bot_lines.insert(0, Line::from(Span::raw("BOT:").green()));

            user_lines.extend(bot_lines);
            user_lines
        })
        .collect();
    let messages_text = Text::from(messages);
    let messages = Paragraph::new(messages_text)
        .wrap(Wrap { trim: true })
        .block(Block::bordered().title("Chat"));
    f.render_widget(messages, messages_area);
}
