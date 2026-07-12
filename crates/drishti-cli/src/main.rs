//! `drishti`: a small CLI for one-off checks. Loads a TOML config (which is the
//! only place models are chosen, never hardcoded), builds a Drishti instance,
//! runs the requested check, and prints the structured result as JSON.

use std::error::Error;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use drishti_core::config::DrishtiConfig;
use drishti_core::Drishti;
use drishti_models::FsSource;
use futures::executor::block_on;

#[derive(Parser)]
#[command(name = "drishti", about = "Content-safety checks: prompt injection, PII, output safety")]
struct Cli {
    /// Path to the TOML configuration that selects the models for each check.
    #[arg(short, long)]
    config: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Prompt-injection check on an input.
    Prompt(InputArgs),
    /// PII detection and redaction on an input.
    Pii(InputArgs),
    /// Output-safety check on a model output.
    Output(InputArgs),
    /// Run every enabled check. The text is used as the prompt; pass --output
    /// to also run the output-safety check on a separate string.
    All {
        #[command(flatten)]
        input: InputArgs,
        #[arg(long)]
        output: Option<String>,
    },
    /// Print the loaded model manifest (ids and hashes) for audit.
    Manifest,
}

/// Input text, given inline with --text or read from a file with --file.
#[derive(Args)]
struct InputArgs {
    #[arg(long, conflicts_with = "file")]
    text: Option<String>,
    #[arg(long)]
    file: Option<PathBuf>,
}

impl InputArgs {
    fn read(&self) -> Result<String, Box<dyn Error>> {
        match (&self.text, &self.file) {
            (Some(t), _) => Ok(t.clone()),
            (None, Some(p)) => Ok(std::fs::read_to_string(p)?),
            (None, None) => Err("provide --text or --file".into()),
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    dotenvy::dotenv().ok();
    let config_text = std::fs::read_to_string(&cli.config)?;
    let config = DrishtiConfig::from_toml_and_env(&config_text)?;

    let source = FsSource::with_optional_cache(config.cache_dir.clone());
    let drishti = Drishti::builder().with_config(config).build(&source)?;

    match cli.command {
        Command::Prompt(input) => {
            let result = block_on(drishti.check_prompt(&input.read()?))?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Pii(input) => {
            let result = block_on(drishti.check_pii(&input.read()?))?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Output(input) => {
            let result = block_on(drishti.check_output(&input.read()?))?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::All { input, output } => {
            let prompt = input.read()?;
            let result = block_on(drishti.check_all(&prompt, output.as_deref()))?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Manifest => {
            println!("{}", serde_json::to_string_pretty(&drishti.model_manifest())?);
        }
    }
    Ok(())
}
