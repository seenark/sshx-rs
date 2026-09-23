pub mod connect;
pub mod discovery;
pub mod doctor;
pub mod mutation;
pub mod output;
pub mod pair;
pub mod permissions;
pub mod session;
pub mod settings;
pub mod tunnel;

/// Version string exposed for clients embedding the reusable `sshx` library.
pub const VERSION: &str = concat!(env!("CARGO_PKG_NAME"), " ", env!("CARGO_PKG_VERSION"));
