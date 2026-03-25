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
    /// Inspect a symbol's edges
    Inspect {
        /// Symbol name to inspect
        symbol: String,
    },
    /// Show edge summary or list edges
    Edges {
        /// Filter by edge kind (e.g., Calls, Imports, Contains)
        #[arg(long)]
        kind: Option<String>,
        /// Max edges to show
        #[arg(long, default_value = "20")]
        limit: usize,
    },
}

fn main() {
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
        Commands::Inspect { ref symbol } => commands::inspect::run(&root, symbol),
        Commands::Edges { ref kind, limit } => commands::edges::run(&root, kind.as_deref(), limit),
    };

    if let Err(e) = result {
        eprintln!("error: {:#}", e);
        std::process::exit(1);
    }
}
