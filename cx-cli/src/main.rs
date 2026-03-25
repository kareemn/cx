mod commands;
mod mcp;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "cx", about = "Code intelligence engine")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Index the current directory
    Init,
    /// Show service structure
    Context,
    /// Search for symbols
    Search {
        /// Search query
        query: String,
    },
}

fn main() {
    // Reset SIGPIPE to default behavior so piping to `head` etc. exits cleanly
    #[cfg(unix)]
    {
        unsafe {
            libc::signal(libc::SIGPIPE, libc::SIG_DFL);
        }
    }

    let cli = Cli::parse();
    let root = std::env::current_dir().expect("failed to get current directory");

    let result = match cli.command {
        Commands::Init => commands::init::run(&root),
        Commands::Context => commands::context::run(&root),
        Commands::Search { ref query } => commands::search::run(&root, query),
    };

    if let Err(e) = result {
        eprintln!("error: {:#}", e);
        std::process::exit(1);
    }
}
