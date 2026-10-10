//! SketchUp `.skp` reader, and a legacy-format (v17) writer in [`write`](mod@write).
//!
//! A Rust port of the reader in [OpenSKP](https://github.com/iamahsanmehmood/openskp)
//! (MIT, Copyright (c) 2024-2026 Ahsan Mehmood). Both container formats are supported:
//!
//! - VFF (SketchUp 2021 and later): a binary header followed by a ZIP holding `model.dat`
//!   (TLV records), material and style XML, and `meta/meta.dat`.
//! - Legacy (SketchUp 3 to 2020): an MFC `CArchive` object stream.
//!
//! Module layout follows OpenSKP's C++ sources so the two can be compared file by file.

mod bytes;
pub mod dump;
mod geometry;
mod legacy;
pub mod model;
mod pages;
mod raw;
mod tlv;
mod vff;
pub mod write;
mod xml;

pub use model::*;
pub use raw::CoEdge;
pub use write::{DefinitionBuilder, SkpBuilder, WriteError};

use std::path::Path;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    /// Not a SketchUp file (bad magic bytes).
    NotSkp,
    /// A read ran past the end of the data.
    Truncated,
    /// Structurally invalid content.
    Malformed(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::NotSkp => f.write_str("not a valid SketchUp file (bad header magic)"),
            Self::Truncated => f.write_str("file is truncated"),
            Self::Malformed(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Reads and parses a `.skp` file.
pub fn read(path: impl AsRef<Path>) -> Result<Model, Error> {
    parse(&std::fs::read(path)?)
}

/// Parses `.skp` file contents.
pub fn parse(data: &[u8]) -> Result<Model, Error> {
    if !vff::valid_header(data) {
        return Err(Error::NotSkp);
    }
    let raw = if vff::is_legacy(data) {
        legacy::parse_legacy(data)?
    } else {
        vff::full_parse(data)?
    };
    Ok(model::build_model(raw))
}

/// Version string from the file header, e.g. `{25.0.575}`, or `unknown`.
pub fn version(data: &[u8]) -> String {
    vff::extract_version(data)
}

/// 13-double SketchUp transforms, as stored on [`Instance::matrix`].
pub mod transform {
    pub use crate::pages::{multiply_matrices, transform_point};
}
