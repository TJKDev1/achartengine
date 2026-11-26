use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ace_host::{HostCall, Limits, Plugin, PluginError, Region, Runtime, copy_in, copy_out};

use crate::{TABLE_ELEMENTS, WasmtimePlugin, WasmtimeRuntime, no_memory};

/// A plugin with a bump allocator, an export that forwards to `host_call`, one that loops
/// forever, and ones that grow its memory and its table.
const PLUGIN: &str = r#"
(module
  (import "ace" "host_call" (func $host_call (param i32 i32) (result i64)))
  (memory (export "memory") 1)
  (global $next (mut i32) (i32.const 8))
  (func (export "alloc") (param $len i32) (result i32)
    (local $ptr i32)
    (local.set $ptr (global.get $next))
    (global.set $next (i32.add (global.get $next) (local.get $len)))
    (local.get $ptr))
  (func (export "free") (param i32 i32))
  (func (export "ask_host") (param i32 i32) (result i64)
    (call $host_call (local.get 0) (local.get 1)))
  (func (export "spin") (loop $forever (br $forever)))
  (func (export "grow") (param $pages i32) (result i32)
    (memory.grow (local.get $pages)))
  (table 0 funcref)
  (func (export "grow_table") (param $elements i32) (result i32)
    (table.grow (ref.null func) (local.get $elements))))
"#;

const PAGE: usize = 64 * 1024;

/// Answers each request with the request reversed, and logs the requests.
struct Reverse {
    requests: Rc<RefCell<Vec<Vec<u8>>>>,
}

impl HostCall for Reverse {
    fn host_call(
        &mut self,
        plugin: &mut dyn Plugin,
        ptr: u32,
        len: u32,
    ) -> Result<u64, PluginError> {
        let request = copy_out(plugin, Region { ptr, len })?;
        let reply: Vec<u8> = request.iter().rev().copied().collect();
        self.requests.borrow_mut().push(request);
        Ok(copy_in(plugin, &reply)?.pack())
    }
}

fn limits() -> Limits {
    Limits {
        fuel: 1_000_000,
        memory_bytes: 2 * PAGE,
        deadline_ticks: None,
    }
}

fn instantiate(runtime: &WasmtimeRuntime, limits: Limits) -> WasmtimePlugin<Reverse> {
    let module = runtime.load(PLUGIN.as_bytes()).unwrap();
    let host = Reverse {
        requests: Rc::default(),
    };
    runtime.instantiate(&module, limits, host).unwrap()
}

#[test]
fn host_call_can_allocate_its_answer_in_the_plugin() {
    let runtime = WasmtimeRuntime::new().unwrap();
    let module = runtime.load(PLUGIN.as_bytes()).unwrap();
    let requests = Rc::default();
    let host = Reverse {
        requests: Rc::clone(&requests),
    };
    let mut plugin = runtime.instantiate(&module, limits(), host).unwrap();

    let request = copy_in(&mut plugin, b"abc").unwrap();
    let results = plugin
        .call("ask_host", &[request.ptr.into(), request.len.into()])
        .unwrap();
    let [answer] = results[..] else {
        panic!("`ask_host` returns one value, got {results:?}");
    };
    let answer = Region::unpack(answer);
    assert_eq!(
        answer.ptr,
        request.ptr + request.len,
        "the host placed its answer with the plugin's bump allocator"
    );
    let reply = copy_out(&mut plugin, answer).unwrap();
    assert_eq!(reply, b"cba", "the host's answer reaches the plugin");
    assert_eq!(
        *requests.borrow(),
        [b"abc"],
        "the host saw the request once"
    );
}

#[test]
fn a_host_call_error_comes_back_unchanged() {
    let runtime = WasmtimeRuntime::new().unwrap();
    let mut plugin = instantiate(&runtime, limits());
    // The host's `copy_out` reads past the end of memory, and the call reports its error.
    let ptr = u32::try_from(PAGE).unwrap();
    assert_eq!(
        plugin.call("ask_host", &[ptr.into(), 1]),
        Err(PluginError::OutOfBounds { ptr, len: 1 }),
        "the host's own error, not a generic trap"
    );
}

#[test]
fn fuel_stops_an_infinite_loop_and_can_be_refilled() {
    let runtime = WasmtimeRuntime::new().unwrap();
    let mut plugin = instantiate(&runtime, limits());
    assert_eq!(
        plugin.call("spin", &[]),
        Err(PluginError::OutOfFuel),
        "the loop runs until fuel is gone"
    );
    assert_eq!(plugin.fuel(), Ok(0), "no fuel is left");
    plugin.set_fuel(1_000).unwrap();
    assert!(
        plugin.call("alloc", &[8]).is_ok(),
        "the plugin works again with new fuel"
    );
    let left = plugin.fuel().unwrap();
    assert!(
        0 < left && left < 1_000,
        "the call used some of the new fuel, not all, got {left}"
    );
}

/// Refuels the plugin from inside `host_call` and answers with the fuel it then has.
struct Refuel;

impl HostCall for Refuel {
    fn host_call(
        &mut self,
        plugin: &mut dyn Plugin,
        _ptr: u32,
        _len: u32,
    ) -> Result<u64, PluginError> {
        plugin.set_fuel(500)?;
        plugin.fuel()
    }
}

#[test]
fn host_call_can_read_and_set_fuel() {
    let runtime = WasmtimeRuntime::new().unwrap();
    let module = runtime.load(PLUGIN.as_bytes()).unwrap();
    let mut plugin = runtime.instantiate(&module, limits(), Refuel).unwrap();
    assert_eq!(
        plugin.call("ask_host", &[0, 0]),
        Ok(vec![500]),
        "the host sees the fuel it set"
    );
}

#[test]
fn a_trap_in_the_start_function_is_a_trap() {
    let runtime = WasmtimeRuntime::new().unwrap();
    let module = runtime
        .load(b"(module (memory (export \"memory\") 1) (func $start unreachable) (start $start))")
        .unwrap();
    assert!(
        matches!(
            runtime.instantiate(&module, limits(), Refuel),
            Err(PluginError::Trap(_))
        ),
        "the module is valid; running it failed"
    );
}

#[test]
fn the_deadline_stops_an_infinite_loop() {
    let runtime = WasmtimeRuntime::new().unwrap();
    // Enough fuel to outlast the deadline by far, but finite: without ticks the call ends
    // with `OutOfFuel` instead of hanging the test.
    let mut plugin = instantiate(
        &runtime,
        Limits {
            fuel: 1 << 32,
            deadline_ticks: Some(1),
            ..limits()
        },
    );
    let ticker = runtime.ticker();
    // Tick in a loop: each call restarts its deadline, so ticks sent before the call don't count.
    let done = Arc::new(AtomicBool::new(false));
    let ticking = std::thread::spawn({
        let done = Arc::clone(&done);
        move || {
            while !done.load(Ordering::Relaxed) {
                ticker.tick();
                std::hint::spin_loop();
            }
        }
    });
    let result = plugin.call("spin", &[]);
    done.store(true, Ordering::Relaxed);
    ticking.join().unwrap();
    assert_eq!(
        result,
        Err(PluginError::DeadlineExceeded),
        "ticks stop the loop"
    );
}

#[test]
fn the_deadline_counts_from_the_start_of_each_call() {
    let runtime = WasmtimeRuntime::new().unwrap();
    let mut plugin = instantiate(
        &runtime,
        Limits {
            deadline_ticks: Some(5),
            ..limits()
        },
    );
    for _ in 0..100 {
        runtime.ticker().tick();
    }
    assert!(
        plugin.call("alloc", &[8]).is_ok(),
        "ticks between calls don't use up the next call's deadline"
    );
}

#[test]
fn without_a_deadline_ticks_never_stop_a_call() {
    let runtime = WasmtimeRuntime::new().unwrap();
    let mut plugin = instantiate(&runtime, limits());
    for _ in 0..100 {
        runtime.ticker().tick();
    }
    assert!(
        plugin.call("alloc", &[8]).is_ok(),
        "`None` means no deadline"
    );
}

#[test]
fn growing_past_the_memory_cap_fails_inside_the_plugin() {
    let runtime = WasmtimeRuntime::new().unwrap();
    let mut plugin = instantiate(&runtime, limits());
    assert_eq!(
        plugin.call("grow", &[1]),
        Ok(vec![1]),
        "the second page fits under the two-page cap"
    );
    assert_eq!(
        plugin.call("grow", &[1]),
        Ok(vec![u64::from(u32::MAX)]),
        "`memory.grow` returns -1 past the cap, and the call goes on"
    );
}

#[test]
fn growing_a_table_past_its_cap_fails_inside_the_plugin() {
    let runtime = WasmtimeRuntime::new().unwrap();
    // `table.grow` costs fuel for each element it adds.
    let mut plugin = instantiate(
        &runtime,
        Limits {
            fuel: u64::MAX,
            ..limits()
        },
    );
    assert_eq!(
        plugin.call("grow_table", &[1]),
        Ok(vec![0]),
        "a table may grow under the cap"
    );
    let cap = u64::try_from(TABLE_ELEMENTS).unwrap();
    assert_eq!(
        plugin.call("grow_table", &[cap]),
        Ok(vec![u64::from(u32::MAX)]),
        "`table.grow` returns -1 past the cap"
    );
}

#[test]
fn calls_are_checked_against_the_export() {
    let runtime = WasmtimeRuntime::new().unwrap();
    let mut plugin = instantiate(&runtime, limits());
    let no_such_export = |name: &str| Err(PluginError::NoSuchExport(name.into()));
    assert_eq!(
        plugin.call("missing", &[]),
        no_such_export("missing"),
        "unknown name"
    );
    assert_eq!(
        plugin.call("alloc", &[]),
        no_such_export("alloc"),
        "too few arguments"
    );
    assert_eq!(
        plugin.call("alloc", &[u64::MAX]),
        no_such_export("alloc"),
        "an argument too large for i32"
    );
}

#[test]
fn modules_outside_the_abi_are_rejected() {
    let runtime = WasmtimeRuntime::new().unwrap();
    assert!(
        matches!(
            runtime.load(b"not wasm"),
            Err(PluginError::InvalidModule(_))
        ),
        "bytes that aren't a module"
    );
    let host = || Reverse {
        requests: Rc::default(),
    };
    let wasi = runtime
        .load(br#"(module (import "wasi" "fd_write" (func)) (memory (export "memory") 1))"#)
        .unwrap();
    assert!(
        matches!(
            runtime.instantiate(&wasi, limits(), host()),
            Err(PluginError::InvalidModule(_))
        ),
        "an import the host doesn't provide"
    );
    let memoryless = runtime.load(b"(module)").unwrap();
    assert_eq!(
        runtime.instantiate(&memoryless, limits(), host()).err(),
        Some(no_memory()),
        "a module without memory"
    );
    let relaxed_simd = b"(module (func (param v128 v128 v128) (result v128)
        (f32x4.relaxed_madd (local.get 0) (local.get 1) (local.get 2))))";
    assert!(
        matches!(
            runtime.load(relaxed_simd),
            Err(PluginError::InvalidModule(_))
        ),
        "relaxed SIMD, whose results differ between machines"
    );
}

#[test]
fn debug_output_hides_wasmtime_internals() {
    let runtime = WasmtimeRuntime::new().unwrap();
    assert_eq!(format!("{runtime:?}"), "WasmtimeRuntime { .. }", "runtime");
    assert_eq!(format!("{:?}", runtime.ticker()), "Ticker { .. }", "ticker");
    assert_eq!(
        format!("{:?}", instantiate(&runtime, limits())),
        "WasmtimePlugin { deadline_ticks: None, .. }",
        "plugin"
    );
}
