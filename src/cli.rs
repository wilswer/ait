use clap::Parser;

#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// System prompt
    #[arg(short, long)]
    pub system_prompt: Option<String>,
    /// Temperature
    #[arg(short, long, default_value = "0.2")]
    pub temperature: f64,
}
