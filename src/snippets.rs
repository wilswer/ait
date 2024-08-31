use std::str::FromStr;

use ratatui::widgets::ListState;

#[derive(Debug)]
pub struct SnippetList {
    pub items: Vec<SnippetItem>,
    pub state: ListState,
}

#[derive(Debug)]
pub struct SnippetItem {
    pub text: String,
    pub selected: bool,
}

impl FromStr for SnippetItem {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(SnippetItem::new(s, false))
    }
}

impl From<String> for SnippetItem {
    fn from(s: String) -> Self {
        SnippetItem::new(&s, false)
    }
}

impl FromIterator<(&'static str, bool)> for SnippetList {
    fn from_iter<I: IntoIterator<Item = (&'static str, bool)>>(iter: I) -> Self {
        let items = iter
            .into_iter()
            .map(|(text, selected)| SnippetItem::new(text, selected))
            .collect();
        let mut state = ListState::default();
        state.select_first();
        Self { items, state }
    }
}

impl SnippetItem {
    pub fn new(snippet: &str, selected: bool) -> Self {
        Self {
            text: snippet.to_string(),
            selected,
        }
    }
}

pub fn find_fenced_code_snippets(messages: Vec<String>) -> Vec<String> {
    let mut snippets = Vec::new();
    let mut in_code_block = false;
    let mut current_snippet = String::new();

    for line in messages {
        if line.trim_start().starts_with("```") {
            // Toggle the state of being inside a code block
            if in_code_block {
                // Code block ends, save the current snippet
                snippets.push(current_snippet.trim_end_matches('\n').to_string());
                current_snippet.clear();
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
// A few tests to ensure the function is working as expected.

mod tests {
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
            "fn main() {
    println!(\"Hello, world!\");
}"
            .to_string(),
            "def main():
    print(\"Hello, world!\")"
                .to_string(),
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
            "    fn main() {
        println!(\"Hello, world!\");
    }",
            "    def main():
        print(\"Hello, world!\")",
        ];
        assert_eq!(
            crate::snippets::find_fenced_code_snippets(messages),
            expected
        );
    }
}
