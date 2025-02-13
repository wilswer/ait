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
    /// Temperature
    #[arg(short, long, value_parser = validate_temperature, default_value = "0.5")]
    pub temperature: f64,
    /// Context input file path. If not provided, reads from stdin
    #[arg(short, long)]
    context: Option<PathBuf>,
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

fn validate_temperature(val: &str) -> Result<f64, String> {
    val.parse::<f64>()
        .map_err(|_| String::from("Value must be a number between 0.0 and 2.0"))
        .and_then(|v| {
            if (0.0..=2.0).contains(&v) {
                Ok(v)
            } else {
                Err(String::from("Value must be a number between 0.0 and 2.0"))
            }
        })
}
