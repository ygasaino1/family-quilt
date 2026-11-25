//! `family-store`: everything that touches the SQLite file.
//!
//! * [`ids`] UUIDv7 ids, strictly increasing within a process
//! * [`dates`] fuzzy dates (`1887`, `1887-04`, `1887-04-12` + qualifier)
//! * [`names`] search keys for names in any script
//! * [`schema`] embedded, numbered migrations
//! * [`store`] / [`people`] / [`links`] / [`graph`] / [`history`] the store layer; every
//!   function takes an [`access::AccessContext`]
//! * [`tree`] / [`crypto`] create/open trees, optional password protection, recovery key, backups
//!
//! Nothing outside this crate writes SQL.

pub mod access;
pub mod crypto;
pub mod dates;
pub mod error;
pub mod graph;
pub mod history;
pub mod ids;
pub mod jalali;
pub mod links;
pub mod model;
pub mod names;
pub mod people;
pub mod review;
pub mod schema;
pub mod store;
pub mod tree;

pub use access::AccessContext;
pub use crypto::{KdfParams, RecoveryKey};
pub use dates::{FuzzyDate, PartialDate, Qualifier};
pub use error::{Result, StoreError};
pub use graph::{Graph, GraphFilter, GraphLink, GraphPerson};
pub use model::*;
pub use people::{NamePatch, SearchQuery};
pub use review::{ChangeRow, ProposalRecords, ProposalRow};
pub use store::Store;
pub use tree::{change_password, protect_tree, reset_password_with_recovery_key, tree_state, unprotect_tree, Credentials, OpenOptions, Protection, TreeState};
