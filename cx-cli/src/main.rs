mod commands;
mod config;
mod indexing;
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
    /// Add another repo to the graph
    Add {
        /// Path to the repo to add
        path: String,
    },
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
    /// Trace execution path from/to a symbol
    Path {
        /// Symbol to trace from (downstream + upstream)
        #[arg(long)]
        from: Option<String>,
        /// Symbol to trace to (find all paths reaching this symbol)
        #[arg(long)]
        to: Option<String>,
        /// Max traversal depth
        #[arg(long, default_value = "20")]
        max_depth: u32,
    },
    /// Show transitive dependencies
    Depends {
        /// Symbol or service name
        symbol: String,
        /// Show upstream (what depends on this) instead of downstream
        #[arg(long)]
        upstream: bool,
        /// Max depth
        #[arg(long, default_value = "10")]
        max_depth: u32,
    },
    /// List all detected network calls and exposed APIs
    Network {
        /// Output as JSON
        #[arg(long)]
        json: bool,
        /// Filter by kind (http, grpc, database, redis, kafka, websocket, sqs, s3, tcp)
        #[arg(long)]
        kind: Option<String>,
        /// Filter by direction (inbound, outbound)
        #[arg(long)]
        direction: Option<String>,
        /// Filter by service/deployable name
        #[arg(long)]
        service: Option<String>,
    },
    /// Start MCP server (JSON-RPC over stdio)
    Mcp,
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
        Commands::Add { ref path } => commands::add::run(&root, path),
        Commands::Context => commands::context::run(&root),
        Commands::Search { ref query } => commands::search::run(&root, query),
        Commands::Inspect { ref symbol } => commands::inspect::run(&root, symbol),
        Commands::Edges { ref kind, limit } => commands::edges::run(&root, kind.as_deref(), limit),
        Commands::Path { ref from, ref to, max_depth } => {
            commands::path::run(&root, from.as_deref(), to.as_deref(), max_depth)
        }
        Commands::Depends {
            ref symbol,
            upstream,
            max_depth,
        } => commands::depends::run(&root, symbol, upstream, max_depth),
        Commands::Network {
            json,
            ref kind,
            ref direction,
            ref service,
        } => commands::network::run(&root, json, kind.as_deref(), direction.as_deref(), service.as_deref()),
        Commands::Mcp => mcp::run(&root),
    };

    if let Err(e) = result {
        eprintln!("error: {:#}", e);
        std::process::exit(1);
    }
}
