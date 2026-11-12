use thiserror::Error;

/// Errors raised while opening or parsing a `.docx` package.
#[derive(Debug, Error)]
pub enum Error {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("not a valid zip package: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("malformed XML in part `{part}`: {source}")]
    Xml {
        part: String,
        #[source]
        source: roxmltree::Error,
    },

    #[error("part `{0}` is neither UTF-8 nor UTF-16")]
    Encoding(String),

    #[error("part `{part}` is larger than the {limit}-byte limit")]
    PartTooLarge { part: String, limit: u64 },

    #[error("the package has no main document part")]
    NoMainDocument,

    #[error("part `{0}` is referenced but missing from the package")]
    MissingPart(String),
}

pub type Result<T> = std::result::Result<T, Error>;
