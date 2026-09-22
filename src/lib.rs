pub mod discovery;
pub mod output;
pub mod settings;

/// Version string exposed for clients embedding the reusable `sshx` library.
pub const VERSION: &str = concat!(env!("CARGO_PKG_NAME"), " ", env!("CARGO_PKG_VERSION"));
