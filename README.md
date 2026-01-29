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

To start the TUI simply run

```bash
ait
```

If you want to provide a custom system prompt, it can be achieved like this:

```bash
ait --system-prompt "You are a helpful, friendly assistant."
```

If you want to add context to your conversation, use the `--context` argument.

```bash
ait --context my_file.txt
```

`ait` can also read from stdin to add context:

```bash
cat my_file.txt | ait
```

A powerful pattern is to use a text serializer such as
[`yek`](https://github.com/bodo-run/yek) and use this as context input:

```bash
yek my_file.txt | ait
```

Or serialize all file in a directory and add as context:

```bash
yek my_dir | ait
```

## Chat history

Chat history is stored as a `sqlite` database (facilitated by the
[`rusqlite`](https://github.com/rusqlite/rusqlite) crate)
in the platform's standard data directory:

- macOS: `~/Library/Application Support/ait/chats.db`
- Linux: `~/.local/share/ait/chats.db`
- Windows: `%APPDATA%\ait\chats.db`

In addition, `ait` will store a log of the latest chat in the platform's cache directory:

- macOS: `~/Library/Caches/ait/latest-chat.log`
- Linux: `~/.cache/ait/latest-chat.log`
- Windows: `%LOCALAPPDATA%\ait\latest-chat.log`

I'm probably the only one using this tool but for users of `ait` version 0.5.1 and
earlier, to keep your old database, simply copy it from the previous location:

```bash
cp ~/.cache/ait/chats.db <new platform specific location according to list above>
```
