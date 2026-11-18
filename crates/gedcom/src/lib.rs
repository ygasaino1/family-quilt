//! GEDCOM import/export for family-quilt.
//!
//! * [`import::import_gedcom`] reads GEDCOM 5.5.1 / 7.0 (UTF-8) into an [`import::ImportPlan`]
//!   (people, links, warnings with line numbers). Nothing touches the database until
//!   [`import::ImportPlan::apply`], which is all-or-nothing. In the app the plan becomes a *proposal*.
//! * [`export::export_gedcom`] writes the verified tree back, including `PEDI`.
//! * `gen_gedcom` (binary) makes synthetic trees for tests and benchmarks.

pub mod date;
pub mod export;
pub mod gen;
pub mod import;
pub mod parse;

pub use export::{export_gedcom, GedcomVersion};
pub use import::{import_gedcom, ApplyReport, ImportPlan, ImportWarning};
pub use parse::Limits;

#[derive(Debug, thiserror::Error)]
pub enum GedError {
    #[error("line {line}: {message}")]
    Syntax { line: usize, message: String },
    #[error("the file is larger than the allowed {limit_bytes} bytes")]
    TooBig { limit_bytes: u64 },
    #[error("the file could not be read: {0}")]
    Io(std::io::Error),
    #[error("this does not look like a GEDCOM file (it must start with \"0 HEAD\")")]
    NotGedcom,
    #[error(transparent)]
    Store(#[from] family_store::StoreError),
}

impl GedError {
    pub fn code(&self) -> &'static str {
        match self {
            GedError::Syntax { .. } => "gedcom_syntax",
            GedError::TooBig { .. } => "gedcom_too_big",
            GedError::Io(_) => "file_error",
            GedError::NotGedcom => "not_gedcom",
            GedError::Store(e) => e.code(),
        }
    }
}
