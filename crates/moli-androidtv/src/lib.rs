//! Android and Google TVs speak two open protocols beside their maker's
//! API:
//!
//! - [`remote`]: Android TV Remote v2 (TLS 6466, the client certificate a
//!   pairing made): the app on screen, launching an app, keys.
//! - [`pairing`]: that pairing (TLS 6467, the code the TV shows), with a
//!   certificate Moli makes.
//! - [`cast`]: Google Cast (TLS 8009): what an app plays (title, picture,
//!   playing or paused) and play / pause.
//!
//! Each runs one connection until it fails; the driver reconnects.
//! [`link`] keeps both connections for a driver (the Philips one, the
//! generic [`AndroidTv`]).

pub mod cast;
mod driver;
pub mod link;
pub mod pairer;
pub mod pairing;
pub mod remote;

pub use driver::{AndroidTv, Config};
/// The protobuf codec, shared with the other protocols.
pub use moli_net::proto;
