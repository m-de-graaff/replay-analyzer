use std::io;

/// Everything that can go wrong while reading a replay.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not a dissect replay file")]
    InvalidFile,
    #[error("folder contains no .rec replay files")]
    InvalidFolder,
    #[error("{0} is an in-progress recording (.tmprec), not a finished replay")]
    TemporaryFile(String),
    #[error("invalid header string separator at offset {0}")]
    InvalidStringSeparator(usize),
    #[error("unexpected end of replay data")]
    UnexpectedEof,
    #[error("header is missing required property `{0}`")]
    MissingProperty(&'static str),
    #[error("header property `{key}` has invalid value {value:?}")]
    InvalidProperty { key: String, value: String },
    #[error("zstd decompression failed: {0}")]
    Decompress(io::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
