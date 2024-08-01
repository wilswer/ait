# AI in the Terminal

`ait` is a terminal user interface for interacting with several
generative large language models from multiple providers.
It uses the [`genai`](https://github.com/jeremychone/rust-genai) crate to
communicate with the model providers.
The TUI is built using the [`ratatui`](https://ratatui.rs) crate.

## Installation

Clone this repository and `cd` to the `ait` directory and run the application using:

```bash
cargo run
```

Install the application by running:

```bash
cargo install --force --path .
```

The binary name is `ait`.

## Usage

The chat interface is modal and starts in the 'normal' mode.
By pressing the `i` key text can be input into the text area.
More information can be found by pressing the `?` key.

## Chat history

Chat history is not yet implemented. `ait` will store a log of the latest chat
in the user's home directory, `~/.cache/ait/latest-chat.log` (not on Windows yet).
