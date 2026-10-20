//! Android and Google TVs speak two open protocols beside their maker's
//! API. The generic driver pairs natively; Philips keeps its existing identity:
//!
//! - [`remote`]: Android TV Remote v2 (TLS 6466, the client certificate a
//!   pairing made): the app on screen, launching an app, keys.
//! - [`pairing`]: human Polo v2 pairing (TLS 6467), bounded framing and RSA identity.
//! - [`cast`]: Google Cast (TLS 8009): what an app plays (title, picture,
//!   playing or paused) and play / pause.
//!
//! Each runs one connection until it fails; the driver reconnects.

pub mod cast;
pub mod pairing;
pub mod remote;
pub mod tracked;

/// The protobuf codec, shared with the other protocols.
pub use moli_net::proto;
