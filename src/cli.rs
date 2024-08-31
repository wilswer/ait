use clap::Parser;

#[derive(Parser, Clone, Debug)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// System prompt
    #[arg(short, long, default_value = "You are a helpful, friendly assistant.")]
    pub system_prompt: String,
    /// Temperature
    #[arg(short, long, value_parser = validate_temperature, default_value = "0.5")]
    pub temperature: f64,
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
