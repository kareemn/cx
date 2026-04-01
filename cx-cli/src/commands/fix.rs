use anyhow::{Context, Result};
use cx_extractors::taint::{Confidence, ResolvedNetworkCall};
use std::path::Path;

/// Run `cx fix` — help correct the graph by generating or checking sink config.
///
/// --init: Generate a starter .cx/config/sinks.toml from all unresolved calls
/// --check: Show what's still unresolved after applying current config
pub fn run(root: &Path, init: bool, check: bool) -> Result<()> {
    if init {
        return generate_sinks_toml(root);
    }

    // Default (and --check): show unresolved calls
    show_unresolved(root, check)
}

/// Generate .cx/config/sinks.toml from network.json entries that are Heuristic or Dynamic.
fn generate_sinks_toml(root: &Path) -> Result<()> {
    let calls = load_network_calls(root);
    if calls.is_empty() {
        eprintln!("No network calls found. Run `cx build` first.");
        return Ok(());
    }

    // Collect unique callees that are heuristic-classified
    let mut seen = std::collections::HashSet::new();
    let mut entries = Vec::new();

    for call in &calls {
        if call.confidence == Confidence::Heuristic || call.confidence == Confidence::LLMClassified {
            let key = call.callee_fqn.clone();
            if seen.insert(key.clone()) {
                entries.push(call);
            }
        }
    }

    if entries.is_empty() {
        eprintln!("All calls are already resolved. Nothing to fix.");
        return Ok(());
    }

    let config_dir = root.join(".cx").join("config");
    std::fs::create_dir_all(&config_dir)
        .context("failed to create .cx/config/")?;
    let config_path = config_dir.join("sinks.toml");

    // Don't overwrite existing
    if config_path.exists() {
        eprintln!(
            "{} already exists. Edit it manually or delete it first.",
            config_path.display()
        );
        eprintln!("Showing what would be generated:\n");
        print_template(&entries);
        return Ok(());
    }

    let content = build_template(&entries);
    std::fs::write(&config_path, &content)
        .with_context(|| format!("failed to write {}", config_path.display()))?;

    eprintln!(
        "Generated {} with {} sink(s)",
        config_path.display(),
        entries.len()
    );
    eprintln!("Review and edit the file, then run `cx build` to apply.");

    Ok(())
}

/// Show unresolved calls — what still needs attention.
fn show_unresolved(root: &Path, verbose: bool) -> Result<()> {
    let calls = load_network_calls(root);
    if calls.is_empty() {
        eprintln!("No network calls found. Run `cx build` first.");
        return Ok(());
    }

    let custom = cx_extractors::custom_sinks::CustomSinkConfig::load(root);
    let has_config = !custom.is_empty();

    let mut heuristic = Vec::new();
    let mut dynamic_source = Vec::new();
    let mut resolved = 0u32;

    for call in &calls {
        match call.confidence {
            Confidence::TypeConfirmed | Confidence::ImportResolved => {
                resolved += 1;
            }
            Confidence::LLMClassified => {
                // Classified by LLM — could be replaced by config
                if is_dynamic_source(&call.address_source) {
                    dynamic_source.push(call);
                } else {
                    resolved += 1; // LLM resolved it well enough
                }
            }
            Confidence::Heuristic => {
                heuristic.push(call);
            }
        }
    }

    let total = calls.len();
    let unresolved = heuristic.len() + dynamic_source.len();

    // Summary
    println!(
        "\x1b[1m{}/{} resolved\x1b[0m  \x1b[2m({} heuristic, {} dynamic source)\x1b[0m",
        resolved,
        total,
        heuristic.len(),
        dynamic_source.len(),
    );

    if has_config {
        println!(
            "\x1b[2mCustom sinks: {} sink(s) loaded from .cx/config/sinks.toml\x1b[0m",
            custom.sinks.len()
        );
    }

    if unresolved == 0 {
        println!("\nAll network calls are resolved.");
        return Ok(());
    }

    // Show heuristic calls
    if !heuristic.is_empty() {
        println!(
            "\n\x1b[1;33mHeuristic\x1b[0m \x1b[2m(needs sink config)\x1b[0m:"
        );
        for call in &heuristic {
            let source = cx_extractors::taint::AddressSource::Dynamic {
                hint: String::new(),
            };
            let chain = crate::indexing::format_address_chain(
                &call.address_source,
            );
            println!(
                "  \x1b[2m{}:{}\x1b[0m  \x1b[33m{}\x1b[0m  callee=\x1b[1m{}\x1b[0m  {}",
                call.file, call.line, call.net_kind.as_str(), call.callee_fqn, chain,
            );
            let _ = source; // suppress unused warning
        }
    }

    // Show dynamic source calls
    if !dynamic_source.is_empty() && verbose {
        println!(
            "\n\x1b[1;31mDynamic source\x1b[0m \x1b[2m(address couldn't be traced)\x1b[0m:"
        );
        for call in &dynamic_source {
            let chain = crate::indexing::format_address_chain(&call.address_source);
            println!(
                "  \x1b[2m{}:{}\x1b[0m  \x1b[33m{}\x1b[0m  callee=\x1b[1m{}\x1b[0m  {}",
                call.file, call.line, call.net_kind.as_str(), call.callee_fqn, chain,
            );
        }
    }

    if !heuristic.is_empty() {
        println!(
            "\n\x1b[2mRun `cx fix --init` to generate .cx/config/sinks.toml with these entries\x1b[0m"
        );
    }

    Ok(())
}

fn is_dynamic_source(source: &cx_extractors::taint::AddressSource) -> bool {
    matches!(source, cx_extractors::taint::AddressSource::Dynamic { .. })
}

fn build_template(calls: &[&ResolvedNetworkCall]) -> String {
    let mut out = String::from(
        "# cx custom sink definitions — teach cx about your repo's network functions\n\
         # Generated by `cx fix --init`. Review and edit before running `cx build`.\n\
         #\n\
         # Fields:\n\
         #   fqn       = function name (short like pgxpool.New or full import path)\n\
         #   category  = http_client|http_server|grpc_client|grpc_server|database|redis|\n\
         #               kafka_producer|kafka_consumer|websocket_client|websocket_server|\n\
         #               sqs|s3|tcp_dial|tcp_listen\n\
         #   addr_arg  = which argument (0-indexed) carries the address/connection string\n\
         #   direction = outbound|inbound\n\n",
    );

    for call in calls {
        out.push_str(&format!(
            "[[sinks]]\n\
             fqn = \"{}\"\n\
             category = \"{}\"    # verify this is correct\n\
             addr_arg = 0          # which arg has the address? check the function signature\n\
             direction = \"outbound\"\n\
             # source: {}:{}\n\n",
            call.callee_fqn,
            call.net_kind.as_str(),
            call.file,
            call.line,
        ));
    }

    out
}

fn print_template(calls: &[&ResolvedNetworkCall]) {
    let content = build_template(calls);
    println!("{}", content);
}

fn load_network_calls(root: &Path) -> Vec<ResolvedNetworkCall> {
    let path = root.join(".cx").join("graph").join("network.json");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    serde_json::from_str(&content).unwrap_or_default()
}
