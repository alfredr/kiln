mod bibliography;
mod build;
mod config;
mod content;
mod dev;
mod init;
mod template;
mod world;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "kiln", about = "A document-native static site generator")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Project root directory (default: current directory)
    #[arg(long, global = true)]
    root: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new kiln site
    Init {
        /// Project name (creates a new directory; omit to use current directory)
        name: Option<String>,
    },
    /// Build the site
    Build {
        /// Include draft posts
        #[arg(long)]
        drafts: bool,
    },
    /// Start a dev server with live reload
    Dev {
        /// Port to serve on
        #[arg(long, default_value_t = 3000)]
        port: u16,
        /// Include draft posts
        #[arg(long)]
        drafts: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let root = cli
        .root
        .unwrap_or_else(|| std::env::current_dir().expect("failed to get current directory"));

    match cli.command {
        Commands::Init { name } => {
            init::init(name.as_deref())?;
        }
        Commands::Build { drafts } => {
            let config = config::Config::load(&root)?;
            build::build(&root, &config, drafts)?;
        }
        Commands::Dev { port, drafts } => {
            let config = config::Config::load(&root)?;
            dev::dev(&root, &config, port, drafts).await?;
        }
    }

    Ok(())
}
