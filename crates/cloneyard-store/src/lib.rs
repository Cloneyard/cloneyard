//! The node's SQLite database: one connection on a thread of its own, migrated on open.

mod clone;
mod hook;
mod schema;
mod store;
mod write;

pub use clone::NewClone;
pub use hook::{NewHookEvent, Refused};
pub use store::Store;
