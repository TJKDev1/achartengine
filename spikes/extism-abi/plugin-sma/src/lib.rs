//! Spike plugin: a `Recompute`-style SMA over a raw `f64` column, plus two misbehaving requests
//! for testing the host's limits. Built on the Extism PDK; see `spike-abi` for the exports.

use std::cell::RefCell;
use std::collections::BTreeMap;

use extism_pdk::{FnResult, host_fn, plugin_fn};
use spike_abi::{ColumnId, HostRequest, HostResponse, Request, Response, decode, encode};

#[host_fn("ace")]
extern "ExtismHost" {
    fn host_call(request: Vec<u8>) -> Vec<u8>;
}

/// Columns the plugin keeps between calls. Extism keeps the instance alive from one call to
/// the next, so this survives until the host drops the plugin.
#[derive(Default)]
struct Columns {
    next_id: u32,
    by_id: BTreeMap<u32, Vec<f64>>,
}

thread_local! {
    static COLUMNS: RefCell<Columns> = RefCell::default();
}

/// Copies raw little-endian `f64`s from the input into a new column and returns its id.
#[plugin_fn]
pub fn put_column(bytes: Vec<u8>) -> FnResult<Vec<u8>> {
    let values = bytes
        .chunks_exact(8)
        .map(|chunk| {
            let mut value = [0; 8];
            value.copy_from_slice(chunk);
            f64::from_le_bytes(value)
        })
        .collect();
    Ok(keep(values).0.to_le_bytes().to_vec())
}

/// Returns a column's raw little-endian bytes and drops it.
#[plugin_fn]
pub fn take_column(id: Vec<u8>) -> FnResult<Vec<u8>> {
    let id: [u8; 4] = id.as_slice().try_into()?;
    let values = COLUMNS
        .with_borrow_mut(|columns| columns.by_id.remove(&u32::from_le_bytes(id)))
        .ok_or_else(|| extism_pdk::Error::msg("no such column"))?;
    Ok(values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect())
}

/// Handles one MessagePack [`Request`] and returns a MessagePack [`Response`].
#[plugin_fn]
pub fn call(request: Vec<u8>) -> FnResult<Vec<u8>> {
    let response = match decode::<Request>(&request) {
        Ok(request) => dispatch(request),
        Err(error) => Response::Error(format!("bad request: {error}")),
    };
    Ok(encode(&response))
}

fn dispatch(request: Request) -> Response {
    match request {
        Request::Noop => Response::Done,
        Request::Sma { period, input } => sma(period, input),
        Request::Drop { column } => {
            COLUMNS.with_borrow_mut(|columns| columns.by_id.remove(&column.0));
            Response::Done
        }
        Request::Spin => spin(),
        Request::Hog { bytes } => hog(bytes),
    }
}

fn sma(period: u32, input: ColumnId) -> Response {
    if period == 0 {
        return Response::Error("period must be at least 1".into());
    }
    let output = COLUMNS.with_borrow(|columns| {
        columns
            .by_id
            .get(&input.0)
            .map(|values| rolling_mean(values, period as usize))
    });
    let Some(output) = output else {
        return Response::Error("no such column".into());
    };
    let rows = output.len() as u32;
    match store_row_count(rows) {
        Ok(()) => Response::Column(keep(output)),
        Err(message) => Response::Error(message),
    }
}

/// NaN until the window is full, then the mean of the last `period` values.
fn rolling_mean(values: &[f64], period: usize) -> Vec<f64> {
    let mut sum = 0.0;
    values
        .iter()
        .enumerate()
        .map(|(i, value)| {
            sum += value;
            if i >= period {
                sum -= values[i - period];
            }
            if i + 1 >= period {
                sum / period as f64
            } else {
                f64::NAN
            }
        })
        .collect()
}

fn keep(values: Vec<f64>) -> ColumnId {
    COLUMNS.with_borrow_mut(|columns| {
        let id = columns.next_id;
        columns.next_id += 1;
        columns.by_id.insert(id, values);
        ColumnId(id)
    })
}

fn spin() -> Response {
    let mut n: u64 = 0;
    loop {
        n = std::hint::black_box(n.wrapping_add(1));
    }
}

fn hog(bytes: u32) -> Response {
    let mut buffer: Vec<u8> = Vec::new();
    match buffer.try_reserve_exact(bytes as usize) {
        Ok(()) => Response::Done,
        Err(_) => Response::Error(format!("could not allocate {bytes} bytes")),
    }
}

/// Stores the row count in host storage, which needs the `storage` permission.
fn store_row_count(rows: u32) -> Result<(), String> {
    let request = HostRequest::StorageSet {
        key: "sma/rows".into(),
        value: rows.to_le_bytes().to_vec(),
    };
    // SAFETY: `host_call` is the host's import; the PDK copies the bytes in and out.
    let reply = unsafe { host_call(encode(&request)) }.map_err(|error| error.to_string())?;
    match decode(&reply).map_err(|error| format!("bad host response: {error}"))? {
        HostResponse::Done => Ok(()),
        // The host checks for this exact text in `check_permission_denied`.
        HostResponse::Denied { permission } => Err(format!("host denied `{permission}`")),
    }
}
