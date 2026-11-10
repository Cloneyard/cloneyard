//! JSON-RPC 2.0 with one message per line, over any byte stream: a Unix socket, a plugin's stdio.

mod client;
mod error;
mod frame;
mod message;
mod router;
mod server;

pub use client::{Client, ClientError};
pub use error::Error;
pub use router::{Handler, Router};
pub use server::serve;
