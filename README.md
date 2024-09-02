# AI in the Terminal

`ait` is a terminal user interface for interacting with several
generative large language models from multiple providers.
It uses the [`genai`](https://github.com/jeremychone/rust-genai) crate to
communicate with the model providers.
The TUI is built using the [`ratatui`](https://ratatui.rs) crate.

## Installation

Installation requires `cargo` to be installed.

```bash
cargo install ait
```

### Manual installation

Clone this repository and `cd` to the `ait` directory and run the application using:

```bash
cargo run
```

Install the application by running:

```bash
cargo install --force --path .
```

The binary name is `ait`.

Binaries are also available for download under [Releases](https://github.com/wilswer/ait/releases).

## Usage

The chat interface is modal and starts in the 'normal' mode.
By pressing the `i` key text can be input into the text area.
More information can be found by pressing the `?` key.
To submit queries to the model providers, you either need to obtain an API key and
set the appropriate environment variable OR you need a running
[Ollama](https://ollama.com/) instance on `http://localhost:11434`.

## Chat history

Chat history is stored as a `sqlite` database (facilitated by the
[`rusqlite`](https://github.com/rusqlite/rusqlite) crate)
in the users cache directory in the home directory (`~/.cache/ait/chats.db`).
In addition, `ait` will store a log of the latest chat
in the user's home directory, `~/.cache/ait/latest-chat.log` on macOS and Linux.
