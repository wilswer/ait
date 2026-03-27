use clap::Parser;
use std::fs::File;
use std::io::{self, Read};
use std::path::PathBuf;

use crossterm::tty::IsTty;

#[derive(Parser, Clone, Debug)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// System prompt
    #[arg(short, long, default_value = "You are a helpful, friendly assistant.")]
    pub system_prompt: String,
    /// Context input file path. If not provided, reads from stdin
    #[arg(short, long)]
    context: Option<PathBuf>,
    /// Ollama host URL (e.g. http://192.168.1.10:11434/v1/). Defaults to http://localhost:11434/v1/
    #[arg(long)]
    pub ollama_host: Option<String>,
}

impl Cli {
    pub fn read(&self) -> io::Result<Option<String>> {
        let content = if self.context.is_none() && io::stdin().is_tty() {
            None
        } else {
            let mut reader: Box<dyn Read> = match &self.context {
                None => Box::new(io::stdin()),
                Some(path) => Box::new(File::open(path)?),
            };
            let mut content = String::new();
            reader.read_to_string(&mut content)?;
            Some(content)
        };
        Ok(content)
    }
}
