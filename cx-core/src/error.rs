/// CX error type. Every public function in cx-core returns Result<T, CxError>.
#[derive(Debug, thiserror::Error)]
pub enum CxError {
    #[error("graph file corrupted: {0}")]
    CorruptGraph(String),

    #[error("graph file version {found} not supported (expected {expected})")]
    VersionMismatch { found: u32, expected: u32 },

    #[error("index not found: run `cx init` first")]
    NoIndex,

    #[error("repo not found: {0}")]
    RepoNotFound(String),

    #[error("symbol not found: {0}")]
    SymbolNotFound(String),

    #[error("parse error in {file}: {message}")]
    ParseError { file: String, message: String },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("config error: {0}")]
    Config(String),
}
