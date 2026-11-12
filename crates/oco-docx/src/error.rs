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

    /// The file is a recognisable format, just not a `.docx` package.
    #[error("not a .docx package: {0}")]
    OtherFormat(OtherFormat),

    #[error("part `{0}` is referenced but missing from the package")]
    MissingPart(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// What a file turned out to be when it is not a `.docx` package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtherFormat {
    /// An OLE compound file: a password-protected document, or a legacy
    /// binary `.doc` with the wrong extension.
    OleCompoundFile,
    /// An OpenDocument file (`.odt` and friends) with the wrong extension.
    OpenDocument,
}

impl std::fmt::Display for OtherFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            OtherFormat::OleCompoundFile => "OLE compound file (a password-protected document or a legacy .doc)",
            OtherFormat::OpenDocument => "OpenDocument file",
        })
    }
}
