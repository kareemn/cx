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
        /// Skip static analysis, send all calls to LLM for classification
        #[arg(long)]
        model_only: bool,
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
  cx trace DATABASE_URL                  full trace with provenance
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
    /// Add a remote repo's pre-built graph (local path or git URL)
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  cx add ../other-service               local path
  cx add /abs/path/to/repo              absolute path
  cx add git@github.com:org/repo.git    clone via git")]
    Add {
        /// Path to repo or git URL
        path: String,
    },
    /// Refresh graphs from all registered remotes
    Pull {
        /// Only pull a specific remote by name
        #[arg(long)]
        name: Option<String>,
    },
    /// Compare network boundaries between current state and a baseline
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  cx diff --save             save current state as baseline
  cx diff                    compare current vs baseline
  cx diff --branch main      compare current vs another branch
  cx diff --json             machine-readable output")]
    Diff {
        /// Save current state as baseline for future diffs
        #[arg(long)]
        save: bool,
        /// Compare against a git branch (checks it out temporarily)
        #[arg(long)]
        branch: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show unresolved network calls and generate sink config
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  cx fix                show what's unresolved
  cx fix --check        show unresolved + dynamic sources
  cx fix --init         generate .cx/config/sinks.toml template")]
    Fix {
        /// Generate .cx/config/sinks.toml from unresolved calls
        #[arg(long)]
        init: bool,
        /// Show detailed unresolved info including dynamic sources
        #[arg(long)]
        check: bool,
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
    /// Manage cx post-commit git hook
    Hook {
        /// Install the post-commit hook
        #[arg(long)]
        install: bool,
        /// Remove the post-commit hook
        #[arg(long)]
        remove: bool,
    },
    /// Install cx skill for Claude Code
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  cx skill                  install to .claude/skills/ in current repo
  cx skill --global         install to ~/.claude/skills/ for all repos")]
    Skill {
        /// Install globally to ~/.claude/skills/ instead of repo-local
        #[arg(long, short)]
        global: bool,
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
        Commands::Build { ref paths, verbose, model_only } => commands::build::run(&root, paths, verbose, model_only),
        Commands::Add { ref path } => commands::add::run(&root, path),
        Commands::Pull { ref name } => commands::add::run_pull(&root, name.as_deref()),
        Commands::Diff { save, ref branch, json } => commands::diff::run(&root, save, branch.as_deref(), json),
        Commands::Fix { init, check } => commands::fix::run(&root, init, check),
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
        Commands::Hook { install, remove } => commands::hook::run(&root, install, remove),
        Commands::Skill { global } => commands::skill::run(global),
        Commands::Mcp => mcp::run(&root),
    };

    if let Err(e) = result {
        eprintln!("error: {:#}", e);
        std::process::exit(1);
    }
}
