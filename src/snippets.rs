use std::str::FromStr;

use ratatui::{
    style::{Color, Style},
    text::{Line, Span, Text},
    widgets::ListState,
};
use syntect::highlighting::{Theme, ThemeSet};
use syntect::{easy::HighlightLines, parsing::SyntaxSet};

const EMBEDDED_THEME: &[u8] = include_bytes!("../catppuccin-mocha.tmTheme");

pub fn load_theme() -> Theme {
    let mut buff = std::io::Cursor::new(EMBEDDED_THEME);
    ThemeSet::load_from_reader(&mut buff).unwrap_or_else(|_| {
        let ts = ThemeSet::load_defaults();
        ts.themes["base16-mocha.dark"].clone()
    })
}

pub fn create_highlighted_code<'a>(
    code: impl Into<String>,
    language: impl Into<String>,
    theme: &Theme,
) -> Text<'a> {
    // Load syntax set and theme
    let code = code.into();
    let language = language.into();
    let ps = SyntaxSet::load_defaults_nonewlines();

    // Get syntax reference for the specified language
    let syntax = ps
        .find_syntax_by_name(&language)
        .unwrap_or_else(|| ps.find_syntax_plain_text());

    // Create highlighter with default theme
    let mut h = HighlightLines::new(syntax, theme);

    // Create highlighted lines
    let code_lines: Vec<Line> = code
        .lines()
        .map(|line| {
            let highlights = h
                .highlight_line(line, &ps)
                .expect("Error highlighting line");

            let spans: Vec<Span> = highlights
                .into_iter()
                .map(|(style, content)| {
                    Span::styled(
                        content.to_string(),
                        Style::default().fg(convert_syntect_color(style.foreground)),
                    )
                })
                .collect();
            Line::from(spans)
        })
        .collect();
    Text::from(code_lines)
}

fn convert_syntect_color(color: syntect::highlighting::Color) -> Color {
    Color::Rgb(color.r, color.g, color.b)
}

#[derive(Debug, Default)]
pub struct SnippetList {
    pub items: Vec<SnippetItem>,
    pub state: ListState,
}

impl SnippetList {
    pub fn clear(&mut self) {
        self.items.clear();
        self.state.select(None);
    }

    pub fn new() -> Self {
        Self::default()
    }
}
#[derive(Debug)]
pub struct SnippetItem {
    pub text: String,
    pub selected: bool,
    pub language: Option<String>,
}

impl FromStr for SnippetItem {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(SnippetItem::new(s, false, None))
    }
}

impl From<String> for SnippetItem {
    fn from(s: String) -> Self {
        SnippetItem::new(&s, false, None)
    }
}

impl FromIterator<(&'static str, bool, Option<String>)> for SnippetList {
    fn from_iter<I: IntoIterator<Item = (&'static str, bool, Option<String>)>>(iter: I) -> Self {
        let items = iter
            .into_iter()
            .map(|(text, selected, language)| SnippetItem::new(text, selected, language))
            .collect();
        let mut state = ListState::default();
        state.select_first();
        Self { items, state }
    }
}

impl SnippetItem {
    pub fn new(snippet: &str, selected: bool, language: Option<String>) -> Self {
        Self {
            text: snippet.to_string(),
            selected,
            language,
        }
    }
}

impl From<CodeSnippet> for SnippetItem {
    fn from(value: CodeSnippet) -> Self {
        Self {
            text: value.code,
            selected: false,
            language: Some(value.language),
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct CodeSnippet {
    pub language: String,
    pub code: String,
}

pub fn find_fenced_code_snippets(messages: Vec<String>) -> Vec<CodeSnippet> {
    let mut snippets = Vec::new();
    let mut in_code_block = false;
    let mut current_snippet = String::new();
    let mut current_language = String::new();

    for line in messages {
        if line.trim_start().starts_with("```") {
            // Toggle the state of being inside a code block
            if in_code_block {
                // Code block ends, save the current snippet
                snippets.push(CodeSnippet {
                    language: current_language.clone(),
                    code: current_snippet.trim_end_matches('\n').to_string(),
                });
                current_snippet.clear();
                current_language.clear();
            } else {
                // Extract language name after ```
                let trimmed = line.trim_start();
                current_language = translate_language_name_to_syntect_name(trimmed[3..].trim());
            }
            in_code_block = !in_code_block;
        } else if in_code_block {
            // Inside a code block, append the line to the current snippet
            current_snippet.push_str(&line);
            current_snippet.push('\n');
        }
    }

    snippets
}

pub fn translate_language_name_to_syntect_name(s: &str) -> String {
    match s {
        // Special cases
        "tex" | "latex" => "LaTeX".to_string(),
        "ocaml" => "OCaml".to_string(),
        "bash" => "Bourne Again Shell (bash)".to_string(),
        // Probably more special cases to come, otherwise just capitalize it
        _ => {
            let mut c = s.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        }
    }
}

// A few tests to ensure the function is working as expected.

#[test]
fn test_find_snippets1() {
    let messages = vec![
        "Hello, world!".to_string(),
        "```rust".to_string(),
        "fn main() {".to_string(),
        "    println!(\"Hello, world!\");".to_string(),
        "}".to_string(),
        "```".to_string(),
        "This is a test.".to_string(),
        "```python".to_string(),
        "def main():".to_string(),
        "    print(\"Hello, world!\")".to_string(),
        "```".to_string(),
    ];
    let expected = vec![
        CodeSnippet {
            language: "Rust".to_string(),
            code: "fn main() {
    println!(\"Hello, world!\");
}"
            .to_string(),
        },
        CodeSnippet {
            language: "Python".to_string(),
            code: "def main():
    print(\"Hello, world!\")"
                .to_string(),
        },
    ];
    assert_eq!(
        crate::snippets::find_fenced_code_snippets(messages),
        expected
    );
}

#[test]
fn test_find_snippets2() {
    let messages = vec![
        "Hello, world!".to_string(),
        "    ```rust".to_string(),
        "    fn main() {".to_string(),
        "        println!(\"Hello, world!\");".to_string(),
        "    }".to_string(),
        "    ```".to_string(),
        "This is a test.".to_string(),
        "    ```python".to_string(),
        "    def main():".to_string(),
        "        print(\"Hello, world!\")".to_string(),
        "    ```".to_string(),
    ];
    let expected = vec![
        CodeSnippet {
            language: "Rust".to_string(),
            code: "    fn main() {
        println!(\"Hello, world!\");
    }"
            .to_string(),
        },
        CodeSnippet {
            language: "Python".to_string(),
            code: "    def main():
        print(\"Hello, world!\")"
                .to_string(),
        },
    ];
    assert_eq!(
        crate::snippets::find_fenced_code_snippets(messages),
        expected
    );
}
// mod tests {
//     #[test]
//     fn test_find_snippets1() {
//         let messages = vec![
//             "Hello, world!".to_string(),
//             "```rust".to_string(),
//             "fn main() {".to_string(),
//             "    println!(\"Hello, world!\");".to_string(),
//             "}".to_string(),
//             "```".to_string(),
//             "This is a test.".to_string(),
//             "```python".to_string(),
//             "def main():".to_string(),
//             "    print(\"Hello, world!\")".to_string(),
//             "```".to_string(),
//         ];
//         let expected = vec![
//             "fn main() {
//     println!(\"Hello, world!\");
// }"
//             .to_string(),
//             "def main():
//     print(\"Hello, world!\")"
//                 .to_string(),
//         ];
//         assert_eq!(
//             crate::snippets::find_fenced_code_snippets(messages),
//             expected
//         );
//     }
//
//     #[test]
//     fn test_find_snippets2() {
//         let messages = vec![
//             "Hello, world!".to_string(),
//             "    ```rust".to_string(),
//             "    fn main() {".to_string(),
//             "        println!(\"Hello, world!\");".to_string(),
//             "    }".to_string(),
//             "    ```".to_string(),
//             "This is a test.".to_string(),
//             "    ```python".to_string(),
//             "    def main():".to_string(),
//             "        print(\"Hello, world!\")".to_string(),
//             "    ```".to_string(),
//         ];
//         let expected = vec![
//             "    fn main() {
//         println!(\"Hello, world!\");
//     }",
//             "    def main():
//         print(\"Hello, world!\")",
//         ];
//         assert_eq!(
//             crate::snippets::find_fenced_code_snippets(messages),
//             expected
//         );
//     }
// }
