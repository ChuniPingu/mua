use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(version, about = "Raster, DDS, and AFB conversion utilities")]
struct Cli {
    #[arg(long, global = true, default_value = "info")]
    log_level: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Validate a supported raster image.
    Check {
        #[arg(short = 's', long)]
        source: PathBuf,
    },
    /// Convert a raster image to a jacket DDS.
    Jacket {
        #[arg(short = 's', long)]
        source: PathBuf,
        #[arg(short = 'd', long)]
        destination: PathBuf,
    },
    /// Convert and inject stage background/effect textures.
    Stage {
        #[arg(short = 'b', long)]
        background: PathBuf,
        #[arg(short = 's', long)]
        template: Option<PathBuf>,
        #[arg(short = 'd', long)]
        destination: PathBuf,
        #[arg(short = 'n', long)]
        notes_field: Option<PathBuf>,
        #[arg(long)]
        fx1: Option<PathBuf>,
        #[arg(long)]
        fx2: Option<PathBuf>,
        #[arg(long)]
        fx3: Option<PathBuf>,
        #[arg(long)]
        fx4: Option<PathBuf>,
    },
    /// Extract embedded DDS chunks.
    ExtractDds {
        #[arg(short = 's', long)]
        source: PathBuf,
        #[arg(short = 'd', long)]
        destination: PathBuf,
    },
    /// Decode a DDS texture to PNG.
    DecodeDds {
        #[arg(short = 's', long)]
        source: PathBuf,
        #[arg(short = 'd', long)]
        destination: PathBuf,
    },
}

fn execute(cli: Cli) -> mua_img::Result<()> {
    match cli.command {
        Command::Check { source } => mua_img::check(&source),
        Command::Jacket {
            source,
            destination,
        } => mua_img::convert_jacket(&source, &destination),
        Command::Stage {
            background,
            template,
            destination,
            notes_field,
            fx1,
            fx2,
            fx3,
            fx4,
        } => mua_img::convert_stage(
            &background,
            &destination,
            &[fx1, fx2, fx3, fx4],
            template.as_deref(),
            notes_field.as_deref(),
        ),
        Command::ExtractDds {
            source,
            destination,
        } => mua_img::extract_dds(&source, &destination).map(|_| ()),
        Command::DecodeDds {
            source,
            destination,
        } => mua_img::decode_dds(&source, &destination),
    }
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let code = if error.use_stderr() { 64 } else { 0 };
            let _ = error.print();
            return ExitCode::from(code);
        }
    };
    let filter = tracing_subscriber::EnvFilter::try_new(&cli.log_level)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();

    match execute(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error);
            ExitCode::from(1)
        }
    }
}
