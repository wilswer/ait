<div align="center">

# AI in the Terminal

[![Crates.io](https://img.shields.io/crates/v/ait.svg)](https://crates.io/crates/ait) [![Built With Ratatui](https://img.shields.io/badge/Built_With_Ratatui-000?logo=ratatui&logoColor=fff)](https://ratatui.rs/)

</div>

`ait` is a terminal user interface for interacting with several generative large
language models from multiple providers. It uses the
[`genai`](https://github.com/jeremychone/rust-genai) crate to communicate with
the model providers. The TUI is built using the [`ratatui`](https://ratatui.rs)
crate.

https://github.com/user-attachments/assets/23a2ed64-2b15-447d-9efe-dfb36bf932fb

## Installation

Installation requires `cargo` to be installed.

```bash
cargo install ait
```

### Manual installation

Clone this repository and `cd` to the `ait` directory and run the application
using:

```bash
cargo run
```

Install the application by running:

```bash
cargo install --force --path .
```

The binary name is `ait`.

Binaries are also available for download under
[Releases](https://github.com/wilswer/ait/releases).

## Usage

The chat interface is modal and starts in the 'normal' mode. By pressing the `i`
key text can be input into the text area. More information can be found by
pressing the `?` key. To submit queries to the model providers, you either need
to obtain an API key and set the appropriate environment variable OR you need a
running [Ollama](https://ollama.com/) instance on `http://localhost:11434`.

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

AIT can also load typed Python functions as local tools. `uv` must be installed and
available on `PATH`:

```bash
ait --python-tools ./tools.py
```

The option may be repeated. A tool file contains public, module-level Python
functions with complete parameter and return annotations plus a docstring:

```python
def word_count(text: str) -> int:
    """Count words in text."""
    return len(text.split())
```

Python dependencies are managed by `uv`. If `pyproject.toml` is placed directly
beside the script, AIT uses that project and its declared dependencies, while
also supplying Pydantic for schema generation. Without an adjacent project file,
only the standard library and AIT's internal Pydantic dependency are available.
Python files execute as trusted local code with the user's permissions; AIT does
not sandbox them.

Python sources can also be configured in the normal config file:

```toml
[python_tools.sources.weather]
script = "/home/alice/tools/weather.py"
name = "Weather"
enabled = true
# Optional overrides:
# project_dir = "/home/alice/tools"
# uv_command = "uv"
# timeout_secs = 30
```

The adjacent project convention is used by default; `project_dir` overrides it.



Chat history is stored as a `sqlite` database (facilitated by the
[`rusqlite`](https://github.com/rusqlite/rusqlite) crate) in the platform's
standard data directory:

- macOS: `~/Library/Application Support/ait/chats.db`
- Linux: `~/.local/share/ait/chats.db`
- Windows: `%APPDATA%\ait\chats.db`

In addition, `ait` will store a log of the latest chat in the platform's cache
directory:

- macOS: `~/Library/Caches/ait/latest-chat.log`
- Linux: `~/.cache/ait/latest-chat.log`
- Windows: `%LOCALAPPDATA%\ait\latest-chat.log`

I'm probably the only one using this tool but for users of `ait` version 0.5.1
and earlier, to keep your old database, simply copy it from the previous
location:

```bash
cp ~/.cache/ait/chats.db <new platform specific location according to list above>
```

## Configuration

AIT can be configured via a `config.toml` file. Please refer to this file for a
[minimal example](./config.toml.example).

This file should be stored in the platform specific location:

- Linux: `$XDG_CONFIG_HOME` or `$HOME`/.config/ait/config.toml, e.g.,
  /home/alice/.config/ait/config.toml
- macOS: `$HOME`/Library/Application Support/ait, e.g.,
  /Users/Alice/Library/Application Support/ait/config.toml
- Windows: `{FOLDERID_RoamingAppData}`\ait\config.toml, e.g.,
  C:\Users\Alice\AppData\Roaming\ait\config.toml

## MCP (Model Context Protocol) servers

`ait` supports the [Model Context Protocol](https://modelcontextprotocol.io) for
extending the assistant with external tools (file access, search, weather, and
so on). MCP servers are declared in `config.toml` under `[mcp.servers.<id>]` and
connect automatically on startup when enabled.

### Configuration

Each server has a stable id (the TOML table key) and uses **either** the
**stdio** transport (a child process) **or** the **http** transport (a remote
URL). Setting both `command` and `url`, or neither, is a configuration error.

```toml
[mcp.servers.filesystem]
name = "Filesystem"
enabled = true
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
env = {}
```

```toml
[mcp.servers.weather]
name = "Weather"
enabled = true
url = "https://weather.example.com/mcp"
api_key = "your-secret-key"
headers = { "X-Custom-Header" = "value" }
```

| Field | Description |
| ----- | ----------- |
| `name` | Optional human-readable display name. Defaults to the server id. |
| `enabled` | Whether the server connects automatically at startup. Defaults to `true`. |
| `command` | Command to spawn for a stdio server (e.g. `npx`, `uvx`). |
| `args` | Arguments passed to the command. |
| `env` | Extra environment variables for the spawned process (where stdio server secrets such as API keys go). |
| `url` | URL of a streamable-http MCP server. |
| `api_key` | API key sent as `Authorization: Bearer <api_key>` on every request to an http server. |
| `headers` | Extra HTTP headers for an http server (e.g. `X-API-Key`). |

### Secrets

Both transports support **environment variable expansion**, so you never need to
store API keys or other secrets verbatim in the config file. References are
expanded at connect time; placeholders stay on disk.

- `${VAR}` → value of `VAR` (error if unset)
- `${VAR:-default}` → value of `VAR`, or `default` if unset
- `$VAR` → value of `VAR` (error if unset)
- `$$` — a literal `$`

```toml
[mcp.servers.kagi]
command = "uvx"
args = ["kagimcp"]
env = { KAGI_API_KEY = "${KAGI_API_KEY}" }   # set KAGI_API_KEY in your shell
```

An unset variable with no default is a hard error, so a missing secret fails
loudly instead of silently sending an empty value to a server.

### Managing servers in-app

- Press `S` in normal mode to open the **server management** view.
- Use `j`/`k` or `Up`/`Down` to move through the configured servers.
- Press `Space` to **toggle** a server's enabled state, connecting or
  disconnecting it live.
- Press `Esc`, `q`, or `S` to return to normal mode.

The footer shows a summary of MCP server status (ready, connecting, failed)
and the connected tools available to the assistant.

### How tools are used

When you submit a message, the assistant can call any tool exposed by the
currently-connected MCP servers. The model decides when a tool is needed; each
call is executed immediately and the result is fed back into the conversation
so the model can continue. Tool calls and results are shown inline in the
thinking trace, so you can watch what the assistant is doing. The conversation
supports up to 12 tool-calling rounds per response to prevent runaway loops.

### Example servers

A few well-known MCP servers to try:

```toml
[mcp.servers.filesystem]
name = "Filesystem"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/sandbox"]

[mcp.servers.everything]
name = "Everything (test)"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-everything"]
```
