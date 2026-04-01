//! User-defined sink overrides loaded from `.cx/config/sinks.toml`.
//!
//! Lets teams teach cx about repo-specific network functions without
//! modifying the built-in sink registry. Custom sinks are checked first
//! (user overrides win), and match on short names like `pgxpool.New`
//! in addition to full FQNs.

use serde::Deserialize;
use std::path::Path;

use crate::sink_registry::{Direction, NetworkCategory};

/// A user-defined network sink override.
#[derive(Debug, Clone, Deserialize)]
pub struct CustomSink {
    /// Function name — can be short (pgxpool.New) or full FQN
    pub fqn: String,
    /// Network category (http_client, database, grpc_server, etc.)
    pub category: String,
    /// Which argument (0-indexed) carries the address/target
    #[serde(default)]
    pub addr_arg: u8,
    /// Traffic direction
    #[serde(default = "default_direction")]
    pub direction: String,
}

/// A user-defined endpoint registration override.
#[derive(Debug, Clone, Deserialize)]
pub struct CustomEndpoint {
    /// Function name
    pub fqn: String,
    /// Which argument carries the route pattern
    #[serde(default)]
    pub pattern_arg: u8,
    /// Which argument carries the handler function
    #[serde(default = "default_handler_arg")]
    pub handler_arg: u8,
}

fn default_direction() -> String {
    "outbound".to_string()
}

fn default_handler_arg() -> u8 {
    1
}

/// Top-level config structure for `.cx/config/sinks.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CustomSinkConfig {
    #[serde(default)]
    pub sinks: Vec<CustomSink>,
    #[serde(default)]
    pub endpoints: Vec<CustomEndpoint>,
}

impl CustomSinkConfig {
    /// Load from `.cx/config/sinks.toml`. Returns empty config if file doesn't exist.
    pub fn load(root: &Path) -> Self {
        let path = root.join(".cx").join("config").join("sinks.toml");
        let Ok(content) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        match toml::from_str(&content) {
            Ok(config) => config,
            Err(e) => {
                eprintln!("Warning: failed to parse {}: {}", path.display(), e);
                Self::default()
            }
        }
    }

    /// Check if there are any custom definitions.
    pub fn is_empty(&self) -> bool {
        self.sinks.is_empty() && self.endpoints.is_empty()
    }
}

impl CustomSink {
    /// Parse the category string into a NetworkCategory.
    pub fn network_category(&self) -> Option<NetworkCategory> {
        match self.category.as_str() {
            "http_client" => Some(NetworkCategory::HttpClient),
            "http_server" => Some(NetworkCategory::HttpServer),
            "grpc_client" => Some(NetworkCategory::GrpcClient),
            "grpc_server" => Some(NetworkCategory::GrpcServer),
            "websocket_client" => Some(NetworkCategory::WebsocketClient),
            "websocket_server" => Some(NetworkCategory::WebsocketServer),
            "kafka_producer" => Some(NetworkCategory::KafkaProducer),
            "kafka_consumer" => Some(NetworkCategory::KafkaConsumer),
            "database" => Some(NetworkCategory::Database),
            "redis" => Some(NetworkCategory::Redis),
            "sqs" => Some(NetworkCategory::Sqs),
            "s3" => Some(NetworkCategory::S3),
            "tcp_dial" => Some(NetworkCategory::TcpDial),
            "tcp_listen" => Some(NetworkCategory::TcpListen),
            _ => None,
        }
    }

    /// Parse the direction string.
    pub fn dir(&self) -> Direction {
        match self.direction.as_str() {
            "inbound" => Direction::Inbound,
            _ => Direction::Outbound,
        }
    }

    /// Check if this custom sink matches a callee.
    /// Matches on: exact FQN, or short name (receiver.method).
    pub fn matches(&self, fqn: &str) -> bool {
        // Exact match
        if self.fqn == fqn {
            return true;
        }
        // Short name match: if custom fqn is "pgxpool.New" and candidate is
        // "github.com/jackc/pgx/v5/pgxpool.New", match by suffix
        if fqn.ends_with(&self.fqn) {
            let prefix = &fqn[..fqn.len() - self.fqn.len()];
            return prefix.is_empty() || prefix.ends_with('/') || prefix.ends_with('.');
        }
        // Reverse: if custom fqn is long and callee is short
        if self.fqn.ends_with(fqn) {
            let prefix = &self.fqn[..self.fqn.len() - fqn.len()];
            return prefix.is_empty() || prefix.ends_with('/') || prefix.ends_with('.');
        }
        false
    }
}

/// Look up a custom sink by FQN. Returns (category, addr_arg_index, direction) if found.
pub fn lookup_custom_sink(
    fqn: &str,
    custom: &CustomSinkConfig,
) -> Option<(NetworkCategory, u8, Direction)> {
    for sink in &custom.sinks {
        if sink.matches(fqn) {
            if let Some(cat) = sink.network_category() {
                return Some((cat, sink.addr_arg, sink.dir()));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_sink_exact_match() {
        let sink = CustomSink {
            fqn: "pgxpool.New".to_string(),
            category: "database".to_string(),
            addr_arg: 1,
            direction: "outbound".to_string(),
        };
        assert!(sink.matches("pgxpool.New"));
        assert!(sink.matches("github.com/jackc/pgx/v5/pgxpool.New"));
        assert!(!sink.matches("other.New"));
    }

    #[test]
    fn custom_sink_parse_category() {
        let sink = CustomSink {
            fqn: "test".to_string(),
            category: "database".to_string(),
            addr_arg: 0,
            direction: "outbound".to_string(),
        };
        assert_eq!(sink.network_category(), Some(NetworkCategory::Database));
    }

    #[test]
    fn lookup_custom() {
        let config = CustomSinkConfig {
            sinks: vec![CustomSink {
                fqn: "pgxpool.New".to_string(),
                category: "database".to_string(),
                addr_arg: 1,
                direction: "outbound".to_string(),
            }],
            endpoints: vec![],
        };
        let result = lookup_custom_sink("pgxpool.New", &config);
        assert!(result.is_some());
        let (cat, arg, dir) = result.unwrap();
        assert_eq!(cat, NetworkCategory::Database);
        assert_eq!(arg, 1);
        assert_eq!(dir, Direction::Outbound);
    }

    #[test]
    fn load_missing_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let config = CustomSinkConfig::load(dir.path());
        assert!(config.is_empty());
    }

    #[test]
    fn load_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let config_dir = dir.path().join(".cx").join("config");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(
            config_dir.join("sinks.toml"),
            r#"
[[sinks]]
fqn = "pgxpool.New"
category = "database"
addr_arg = 1

[[sinks]]
fqn = "internal/bus.Publish"
category = "kafka_producer"
"#,
        )
        .unwrap();

        let config = CustomSinkConfig::load(dir.path());
        assert_eq!(config.sinks.len(), 2);
        assert_eq!(config.sinks[0].fqn, "pgxpool.New");
        assert_eq!(config.sinks[0].addr_arg, 1);
        assert_eq!(config.sinks[1].category, "kafka_producer");
    }
}
