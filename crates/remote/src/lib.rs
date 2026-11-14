//! Remote filesystems (FTP / FTPS for now) for twinyard.
//!
//! Panels address remote dirs as `ftp://<connection-id>/abs/path`; the
//! connection id points into `connections.json` (host, TLS mode, user…).

pub mod conn;
pub mod creds;
pub mod engine;
pub mod ftp;
pub mod manager;
pub mod names;
pub mod path;
pub mod search;
pub mod tc;
mod tls;
mod tlsio;

pub use manager::{ConnectResult, Remote};
pub use path::is_remote;
