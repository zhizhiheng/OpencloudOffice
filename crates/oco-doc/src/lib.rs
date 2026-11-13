//! The binary Word format (`.doc`, Word 97 to 2003) turned into the
//! document form.
//!
//! A `.doc` file becomes a WordprocessingML package, the same thing a
//! `.docx` file is, so that it is read, laid out and drawn by the code that
//! shows Word documents, and can be saved as one.
//!
//! This is the skeleton: the format is not read yet.

use std::fmt;

/// Why a file could not be turned into a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Not a compound file, or one without a Word document in it.
    NotWord,
    /// The document is protected with a password.
    Encrypted,
    /// Written by Word 95 or earlier, whose format is another one.
    TooOld,
    /// The file breaks the format; the text says where.
    Damaged(&'static str),
    /// A valid file that uses something not handled.
    Unsupported(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotWord => f.write_str("not a Word document"),
            Error::Encrypted => f.write_str("the document is protected with a password"),
            Error::TooOld => f.write_str("written by Word 95 or earlier, which is not read"),
            Error::Damaged(what) => write!(f, "damaged Word document: {what}"),
            Error::Unsupported(what) => write!(f, "unsupported Word document: {what}"),
        }
    }
}

impl std::error::Error for Error {}

/// Turns a `.doc` file into a `.docx` package.
pub fn to_docx(_bytes: &[u8]) -> Result<Vec<u8>, Error> {
    Err(Error::Unsupported("the binary Word format is not read yet"))
}
