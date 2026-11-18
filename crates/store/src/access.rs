//! The access context (section 16, "reserved for later").
//!
//! Every store function takes one. In v1 there is exactly one value, `Everything`.
//! A later per-vault feature adds values here and filters in **one place**
//! ([`AccessContext::row_filter`]) instead of hunting for SQL across the code base.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AccessContext {
    /// Sees all data (v1: the only kind of context).
    Everything,
}

impl AccessContext {
    /// SQL condition limiting which rows of the table aliased `alias` this
    /// context may see. v1: all rows (all rows have `vault_id IS NULL`).
    pub(crate) fn row_filter(&self, alias: &str) -> String {
        match self {
            AccessContext::Everything => {
                let _ = alias;
                "1 = 1".to_string()
            }
        }
    }

    /// True if answers built from this context may be missing private data.
    /// Always false in v1; relationship output carries it as `incomplete`.
    pub fn is_incomplete_view(&self) -> bool {
        match self {
            AccessContext::Everything => false,
        }
    }
}
