use clap::Parser;

#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// System prompt
    #[arg(short, long)]
    pub system_prompt: Option<String>,
}
