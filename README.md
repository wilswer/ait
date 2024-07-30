# Generative AI in the Terminal

`gait` is a terminal user interface for interacting with several providers of generative large language models.
It uses the [`genai`](https://github.com/jeremychone/rust-genai) crate to communicate with the model providers.
The TUI is built using the [`ratatui`](whttps://ratatui.rs/) crate. crate.

## Installation

Clone this repository and `cd` to the `gait` directory and run the application using
```
cargo run
```
Install the application by running
```
cargo install --force --path .
```

## Usage
The chat interface is modal and starts in the 'normal' mode.
By pressing the `i` key text can be input into the text area.
More information can be found by pressing the `?` key.
