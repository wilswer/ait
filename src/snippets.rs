use std::str::FromStr;

use ratatui::{
    style::{Color, Style},
    text::{Line, Span, Text},
    widgets::ListState,
};
use syntect::highlighting::{Theme, ThemeSet};
use syntect::{easy::HighlightLines, parsing::SyntaxSet};

pub const EMBEDDED_THEME: &[&[u8]; 2] = &[
    include_bytes!("../themes/thorn-dark-warm.tmTheme"),
    include_bytes!("../themes/catppuccin-mocha.tmTheme"),
];

pub fn load_theme(theme_idx: usize) -> Theme {
    let mut buff = std::io::Cursor::new(EMBEDDED_THEME[theme_idx]);
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
    /// Nesting depth: 0 = top-level block, 1 = inside another block, etc.
    pub depth: usize,
}

/// A parsed segment of a message: either plain text or a fenced code block.
///
/// `language` holds the raw tag from the opening fence (e.g. `"rust"`), not the
/// syntect-translated name.  `indent` is the number of leading spaces on the
/// fence line, which callers may use for display.
pub enum MessageSegment {
    Text(String),
    Code {
        language: String,
        code: String,
        indent: usize,
        /// Nesting depth: 0 = top-level, 1 = inside another code block, etc.
        depth: usize,
    },
}

/// Parse `text` into an ordered sequence of [`MessageSegment`]s, handling
/// arbitrarily nested fenced code blocks.
///
/// When a fenced block is opened inside another fenced block the opening/closing
/// fence lines are included verbatim in the outer block's content, and the inner
/// block is also emitted as its own segment with a higher `depth`.
pub fn parse_message_segments(text: &str) -> Vec<MessageSegment> {
    let mut segments: Vec<MessageSegment> = Vec::new();
    // Stack entries: (raw_language, accumulated_code, indent, segments_index)
    let mut stack: Vec<(String, String, usize, usize)> = Vec::new();
    let mut current_text = String::new();

    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(after_backticks) = trimmed.strip_prefix("```") {
            if !stack.is_empty() && after_backticks.is_empty() {
                // Closing fence: finalise the innermost block.
                let (lang, code, indent, idx) = stack.pop().unwrap();
                segments[idx] = MessageSegment::Code {
                    language: lang,
                    code: code.trim_end_matches('\n').to_string(),
                    indent,
                    depth: stack.len(), // depth at which this block was opened
                };
                // Append the closing fence line to the outer block (if any).
                if let Some((_, outer_code, _, _)) = stack.last_mut() {
                    outer_code.push_str(line);
                    outer_code.push('\n');
                }
            } else if !after_backticks.is_empty() {
                // Opening fence: append this line to every already-open block.
                for (_, code, _, _) in stack.iter_mut() {
                    code.push_str(line);
                    code.push('\n');
                }
                // Flush any accumulated plain text (only possible at depth 0).
                if !current_text.is_empty() {
                    segments.push(MessageSegment::Text(std::mem::take(&mut current_text)));
                }
                let indent = line.len() - trimmed.len();
                let idx = segments.len();
                // Reserve a slot; filled in when the block closes.
                segments.push(MessageSegment::Text(String::new()));
                stack.push((after_backticks.to_string(), String::new(), indent, idx));
            }
            // A bare ``` at depth 0 is ignored.
        } else if !stack.is_empty() {
            // Content line: append to every open block (outer accumulates nested content).
            for (_, code, _, _) in stack.iter_mut() {
                code.push_str(line);
                code.push('\n');
            }
        } else {
            current_text.push_str(line);
            current_text.push('\n');
        }
    }

    if !current_text.is_empty() {
        segments.push(MessageSegment::Text(current_text));
    }

    segments
}

pub fn find_fenced_code_snippets(messages: Vec<String>) -> Vec<CodeSnippet> {
    parse_message_segments(&messages.join("\n"))
        .into_iter()
        .filter_map(|seg| match seg {
            MessageSegment::Code {
                language,
                code,
                depth,
                ..
            } => Some(CodeSnippet {
                language: translate_language_name_to_syntect_name(Some(&language)),
                code,
                depth,
            }),
            _ => None,
        })
        .collect()
}

pub fn translate_language_name_to_syntect_name(s: Option<&str>) -> String {
    if let Some(lang) = s {
        match lang {
            // Special cases
            "tex" | "latex" => "LaTeX".to_string(),
            "ocaml" => "OCaml".to_string(),
            "bash" | "sh" => "Bourne Again Shell (bash)".to_string(),
            "sql" => "SQL".to_string(),
            "json" => "JSON".to_string(),
            "yaml" => "YAML".to_string(),
            "css" => "CSS".to_string(),
            "html" => "HTML".to_string(),
            "javascript" => "JavaScript".to_string(),
            // Probably more special cases to come, otherwise just capitalize it
            _ => {
                let mut c = lang.chars();
                match c.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                }
            }
        }
    } else {
        "Plain Text".to_string() // Default to plain text if nothing given
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
            depth: 0,
        },
        CodeSnippet {
            language: "Python".to_string(),
            code: "def main():
    print(\"Hello, world!\")"
                .to_string(),
            depth: 0,
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
            depth: 0,
        },
        CodeSnippet {
            language: "Python".to_string(),
            code: "    def main():
        print(\"Hello, world!\")"
                .to_string(),
            depth: 0,
        },
    ];
    assert_eq!(
        crate::snippets::find_fenced_code_snippets(messages),
        expected
    );
}

#[test]
fn test_nested_snippets() {
    let messages = vec![
        "```markdown".to_string(),
        "# Hello, world!".to_string(),
        "```rust".to_string(),
        "fn main() {".to_string(),
        "    println!(\"Hello, world!\");".to_string(),
        "}".to_string(),
        "```".to_string(),
        "# This is a test.".to_string(),
        "```python".to_string(),
        "def main():".to_string(),
        "    print(\"Hello, world!\")".to_string(),
        "```".to_string(),
        "```".to_string(),
    ];
    let expected = vec![
        CodeSnippet {
            language: "Markdown".to_string(),
            code: "# Hello, world!
```rust
fn main() {
    println!(\"Hello, world!\");
}
```
# This is a test.
```python
def main():
    print(\"Hello, world!\")
```"
            .to_string(),
            depth: 0,
        },
        CodeSnippet {
            language: "Rust".to_string(),
            code: "fn main() {
    println!(\"Hello, world!\");
}"
            .to_string(),
            depth: 1,
        },
        CodeSnippet {
            language: "Python".to_string(),
            code: "def main():
    print(\"Hello, world!\")"
                .to_string(),
            depth: 1,
        },
    ];
    assert_eq!(
        crate::snippets::find_fenced_code_snippets(messages),
        expected
    );
}
