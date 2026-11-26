//! Messages between the spike host and plugin.
//!
//! The ABI has three plugin exports and one host import, all passing `(ptr, len)` pairs into the
//! plugin's linear memory:
//!
//! - `alloc(len) -> ptr` and `free(ptr, len)`: 8-byte aligned buffers, so `f64` columns can be
//!   read in place.
//! - `call(ptr, len) -> packed`: the host writes a MessagePack [`Request`] and gets back a
//!   [`Response`] buffer, which the plugin allocated and the host frees.
//! - `host_call(ptr, len) -> packed`: the plugin sends a [`HostRequest`] and gets back a
//!   [`HostResponse`], which the host allocates in plugin memory through `alloc`.
//!
//! `packed` is `ptr << 32 | len`. Control messages are MessagePack; column data never is. A
//! column travels as a [`ColumnRef`] pointing at raw little-endian values.

use serde::{Deserialize, Serialize};

/// Alignment of every buffer `alloc` returns.
pub const ALIGN: usize = 8;

/// A column of `len` little-endian `f64`s at `ptr` in plugin memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnRef {
    pub ptr: u32,
    /// Counts values, not bytes.
    pub len: u32,
}

/// What the host asks a plugin to do.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum Request {
    /// Does nothing, to measure the cost of a call.
    Noop,
    /// Simple moving average of `input`. The plugin also stores the row count through
    /// `host_call`, which exercises the permission check.
    Sma { period: u32, input: ColumnRef },
    /// Loops forever, to test fuel limits.
    Spin,
    /// Tries to allocate `bytes`, to test memory limits.
    Hog { bytes: u32 },
}

/// The plugin's answer.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum Response {
    /// The request succeeded and has nothing to return.
    Done,
    /// The output column; the host frees it with `free(ptr, len * 8)`.
    Column(ColumnRef),
    /// The request failed, with a message for the log.
    Error(String),
}

/// What a plugin asks the host to do.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum HostRequest {
    /// Needs the `storage` permission.
    StorageSet { key: String, value: Vec<u8> },
}

/// The host's answer.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum HostResponse {
    /// The request succeeded and has nothing to return.
    Done,
    /// The plugin's manifest doesn't grant the permission this request needs.
    Denied { permission: String },
}

/// Packs a buffer into the single `u64` that `call` and `host_call` return.
#[must_use]
pub fn pack(ptr: u32, len: u32) -> u64 {
    (u64::from(ptr) << 32) | u64::from(len)
}

/// Reverses [`pack`].
#[must_use]
pub fn unpack(packed: u64) -> (u32, u32) {
    ((packed >> 32) as u32, packed as u32)
}

/// Encodes a message as MessagePack.
///
/// # Panics
///
/// If encoding fails, which the types above can't: they hold only strings, integers and bytes.
/// Spike code; the real ABI would return the error.
#[must_use]
pub fn encode<T: Serialize>(message: &T) -> Vec<u8> {
    rmp_serde::to_vec(message).expect("message types always encode")
}

/// Decodes a MessagePack message.
///
/// # Errors
///
/// If `bytes` isn't a valid encoding of `T`.
pub fn decode<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> Result<T, rmp_serde::decode::Error> {
    rmp_serde::from_slice(bytes)
}
