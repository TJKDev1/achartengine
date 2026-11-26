//! Messages between the spike host and plugin.
//!
//! Extism moves every input and output through its own kernel memory, and the host can't reach
//! the plugin's linear memory. So instead of pointers, columns are kept by the plugin and named by
//! a [`ColumnId`]. The plugin has three exports, each taking and returning bytes:
//!
//! - `put_column`: the input is raw little-endian `f64`s, which the plugin copies into a column
//!   it keeps. The output is the new [`ColumnId`] as 4 little-endian bytes.
//! - `take_column`: the input is a [`ColumnId`]; the plugin returns that column's raw bytes and
//!   drops it.
//! - `call`: a MessagePack [`Request`] in, a MessagePack [`Response`] out.
//!
//! The host provides one import, `ace::host_call`, which takes a MessagePack [`HostRequest`] and
//! returns a [`HostResponse`]. Control messages are MessagePack; column data never is.

use serde::{Deserialize, Serialize};

/// Names a column the plugin keeps between calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnId(pub u32);

/// What the host asks a plugin to do.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum Request {
    /// Does nothing, to measure the cost of a call.
    Noop,
    /// Simple moving average of `input`. The plugin also stores the row count through
    /// `host_call`, which exercises the permission check.
    Sma { period: u32, input: ColumnId },
    /// Drops a column the host no longer needs.
    Drop { column: ColumnId },
    /// Loops forever, to test fuel and time limits.
    Spin,
    /// Tries to allocate `bytes`, to test memory limits.
    Hog { bytes: u32 },
}

/// The plugin's answer.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum Response {
    /// The request succeeded and has nothing to return.
    Done,
    /// The output column, kept by the plugin until the host takes or drops it.
    Column(ColumnId),
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
