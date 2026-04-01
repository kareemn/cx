mod commands;
mod config;
mod graph_index;
mod indexing;
mod mcp;
mod overlay;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "cx", about = "Code intelligence engine")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build the graph for one or more paths
    Build {
        /// Paths to the repositories to build (defaults to current directory)
        paths: Vec<String>,
        /// Show detailed LSP and LLM classification progress
        #[arg(long, short)]
        verbose: bool,
    },
    /// Trace the lineage of a network call or configuration variable
    #[command(after_help = "\x1b[1mTarget syntax:\x1b[0m
  env:DATABASE_URL       trace a specific env var
  env:AZURE_*            glob -- all env vars matching AZURE_*
  env:*                  all env vars (compact summary)
  call:client.go:Dial    trace a call site in a file
  writer.go:27           trace the function at a file:line
  pgxpool.New            trace callee from network.json (external lib)
  NewAzureASR            trace a function (fuzzy suggests on miss)

\x1b[1mNode types in the graph:\x1b[0m
  Symbol      functions, methods, variables
  Resource    env vars (UPPER_CASE), connection targets (redis, kafka)
  Endpoint    HTTP routes, gRPC services
  Module      packages, modules (Go package, Python module)
  Deployable  services, containers

\x1b[1mEdge types you'll see in traces:\x1b[0m
  Calls       function calls another function
  Configures  function reads an env var / config value
  Connects    function makes a network connection
  Resolves    env var resolves to a connection target
  Contains    module contains a symbol
  Imports     file imports a module
  DependsOn   cross-repo dependency

\x1b[1mExamples:\x1b[0m
  cx trace AZURE_SPEECH_KEY              full trace with provenance
  cx trace 'env:*'                       overview of all env vars
  cx trace 'env:*_ADDR'                  all address env vars
  cx trace pgxpool.New                   trace a database call (resolves via network.json)
  cx trace writer.go:27                  trace the function at a specific line")]
    Trace {
        /// Target to trace (env:VAR, call:file:Func, or symbol name)
        target: String,
        /// Show only upstream paths (who feeds into this?)
        #[arg(long)]
        upstream: bool,
        /// Show only downstream paths (what does this feed into?)
        #[arg(long)]
        downstream: bool,
        /// Max traversal depth
        #[arg(long, default_value = "20")]
        max_depth: u32,
        /// Output as JSON
        #[arg(long)]
        json: bool,
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
        /// Show only local repo data (exclude remote repos)
        #[arg(long)]
        local_only: bool,
        /// Include test, archive, example, and vendor files (excluded by default)
        #[arg(long)]
        include_all: bool,
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
        Commands::Build { ref paths, verbose } => commands::build::run(&root, paths, verbose),
        Commands::Trace {
            ref target,
            upstream,
            downstream,
            max_depth,
            json,
        } => commands::trace::run(&root, target, upstream, downstream, max_depth, json),
        Commands::Network {
            json,
            ref kind,
            ref direction,
            ref service,
            local_only,
            include_all,
        } => commands::network::run(
            &root,
            json,
            kind.as_deref(),
            direction.as_deref(),
            service.as_deref(),
            local_only,
            include_all,
        ),
        Commands::Mcp => mcp::run(&root),
    };

    if let Err(e) = result {
        eprintln!("error: {:#}", e);
        std::process::exit(1);
    }
}
