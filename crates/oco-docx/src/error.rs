use std::fmt;

use crate::{xmltree, zip};

/// Errors raised while opening or parsing a `.docx` package.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Zip(zip::Error),
    Xml {
        part: String,
        source: xmltree::Error,
    },
    Encoding(String),
    PartTooLarge {
        part: String,
        limit: u64,
    },
    NoMainDocument,
    /// The file is a recognisable format, just not a `.docx` package.
    OtherFormat(OtherFormat),
    MissingPart(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(error) => write!(f, "i/o error: {error}"),
            Error::Zip(error) => write!(f, "not a valid zip package: {error}"),
            Error::Xml { part, source } => write!(f, "malformed XML in part `{part}`: {source}"),
            Error::Encoding(part) => write!(f, "part `{part}` is neither UTF-8 nor UTF-16"),
            Error::PartTooLarge { part, limit } => write!(f, "part `{part}` is larger than the {limit}-byte limit"),
            Error::NoMainDocument => f.write_str("the package has no main document part"),
            Error::OtherFormat(format) => write!(f, "not a .docx package: {format}"),
            Error::MissingPart(part) => write!(f, "part `{part}` is referenced but missing from the package"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(error) => Some(error),
            Error::Zip(error) => Some(error),
            Error::Xml { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Error::Io(error)
    }
}

impl From<zip::Error> for Error {
    fn from(error: zip::Error) -> Self {
        match error {
            zip::Error::Io(error) => Error::Io(error),
            other => Error::Zip(other),
        }
    }
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

impl fmt::Display for OtherFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            OtherFormat::OleCompoundFile => "OLE compound file (a password-protected document or a legacy .doc)",
            OtherFormat::OpenDocument => "OpenDocument file",
        })
    }
}
