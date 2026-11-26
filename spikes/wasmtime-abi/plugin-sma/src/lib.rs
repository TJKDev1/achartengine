//! Spike plugin: a `Recompute`-style SMA over a raw `f64` column, plus two misbehaving requests
//! for testing the host's limits. See `spike-abi` for the ABI.

use std::alloc::{Layout, alloc as raw_alloc, dealloc};

use spike_abi::{
    ALIGN, ColumnRef, HostRequest, HostResponse, Request, Response, decode, encode, pack, unpack,
};

#[link(wasm_import_module = "ace")]
unsafe extern "C" {
    fn host_call(ptr: u32, len: u32) -> u64;
}

/// Returns an 8-byte aligned buffer of `len` bytes, or 0 if memory is exhausted.
#[unsafe(no_mangle)]
pub extern "C" fn alloc(len: u32) -> u32 {
    let Ok(layout) = Layout::from_size_align(len.max(1) as usize, ALIGN) else {
        return 0;
    };
    // SAFETY: the layout has a non-zero size.
    unsafe { raw_alloc(layout) as u32 }
}

/// Frees a buffer from [`alloc`].
///
/// # Safety
///
/// `ptr` and `len` must come from one earlier `alloc` call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn free(ptr: u32, len: u32) {
    if let Ok(layout) = Layout::from_size_align(len.max(1) as usize, ALIGN) {
        // SAFETY: the caller passes a pointer and size from `alloc`.
        unsafe { dealloc(ptr as *mut u8, layout) }
    }
}

/// Handles one [`Request`] and returns a packed [`Response`] buffer.
///
/// # Safety
///
/// `ptr` and `len` must describe a readable buffer in this module's memory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn call(ptr: u32, len: u32) -> u64 {
    // SAFETY: the host wrote `len` bytes at `ptr`.
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    let response = match decode::<Request>(bytes) {
        Ok(request) => dispatch(request),
        Err(error) => Response::Error(format!("bad request: {error}")),
    };
    into_plugin_buffer(&encode(&response))
}

fn dispatch(request: Request) -> Response {
    match request {
        Request::Noop => Response::Done,
        Request::Sma { period, input } => sma(period, input),
        Request::Spin => spin(),
        Request::Hog { bytes } => hog(bytes),
    }
}

fn sma(period: u32, input: ColumnRef) -> Response {
    if period == 0 {
        return Response::Error("period must be at least 1".into());
    }
    let Some(out_bytes) = input.len.checked_mul(8) else {
        return Response::Error("column too large".into());
    };
    let out_ptr = alloc(out_bytes);
    if out_ptr == 0 {
        return Response::Error("out of memory".into());
    }
    // SAFETY: the host allocated the input with `alloc` and filled `len` values, and `alloc`
    // just returned room for `len` aligned output values.
    let (values, out) = unsafe {
        (
            std::slice::from_raw_parts(input.ptr as *const f64, input.len as usize),
            std::slice::from_raw_parts_mut(out_ptr as *mut f64, input.len as usize),
        )
    };
    rolling_mean(values, period as usize, out);

    match store_row_count(input.len) {
        Ok(()) => Response::Column(ColumnRef {
            ptr: out_ptr,
            len: input.len,
        }),
        Err(message) => {
            // SAFETY: `out_ptr` came from `alloc` with this size.
            unsafe { free(out_ptr, out_bytes) };
            Response::Error(message)
        }
    }
}

/// NaN until the window is full, then the mean of the last `period` values.
fn rolling_mean(values: &[f64], period: usize, out: &mut [f64]) {
    let mut sum = 0.0;
    for (i, (value, slot)) in values.iter().zip(out.iter_mut()).enumerate() {
        sum += value;
        if i >= period {
            sum -= values[i - period];
        }
        *slot = if i + 1 >= period {
            sum / period as f64
        } else {
            f64::NAN
        };
    }
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
    match ask_host(&request)? {
        HostResponse::Done => Ok(()),
        // The host checks for this exact text in `check_permission_denied`.
        HostResponse::Denied { permission } => Err(format!("host denied `{permission}`")),
    }
}

/// Sends `request` through the `host_call` import and decodes the reply.
fn ask_host(request: &HostRequest) -> Result<HostResponse, String> {
    let bytes = encode(request);
    // SAFETY: `host_call` only reads `bytes` and returns a buffer it allocated through `alloc`.
    let (ptr, len) = unpack(unsafe { host_call(bytes.as_ptr() as u32, bytes.len() as u32) });
    // SAFETY: the host wrote `len` bytes at `ptr`.
    let reply = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    let response = decode(reply).map_err(|error| format!("bad host response: {error}"));
    // SAFETY: the host allocated the reply with `alloc(len)`.
    unsafe { free(ptr, len) };
    response
}

/// Copies `bytes` into a new buffer and packs it for returning from `call`. The host frees it.
/// Traps if memory is exhausted.
fn into_plugin_buffer(bytes: &[u8]) -> u64 {
    let len = bytes.len() as u32;
    let ptr = alloc(len);
    if ptr == 0 {
        // There is no buffer to report the failure in, so trap and let the host see it.
        core::arch::wasm32::unreachable();
    }
    // SAFETY: `alloc` returned `len` writable bytes.
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len()) };
    pack(ptr, len)
}
