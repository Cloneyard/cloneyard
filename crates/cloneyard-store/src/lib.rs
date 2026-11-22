//! The node's SQLite database: one connection on a thread of its own, migrated on open.

mod schema;
mod store;
mod write;

pub use store::Store;
