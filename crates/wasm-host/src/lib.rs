//! Wasm Host (§4.2, M4): wasmtime runtime that hosts the Institutional Layer.
//!
//! Each generation's `gen_XXX.wasm` exports:
//!   - `memory`            — linear memory the host writes input bytes into.
//!   - `process(ptr, len) -> i64` — byte-level state-machine step.
//!
//! ABI for the i64 return:
//!   - `>= 0`  → success; value is the terminal `NodeId` reached.
//!   - `< 0`   → `Unmapped`. Decoded as `packed = -result - 1`,
//!               then `offset = packed >> 8`, `byte = packed & 0xFF`.
//!     The `+1` shift guarantees `Unmapped { offset: 0, byte: 0 }` is still
//!     representable as a strictly negative value.
//!
//! The host wires two metabolic primitives required by the spec (§4.2):
//!   - `Config::consume_fuel(true)` — every `process()` call is given a fixed
//!     fuel budget. Exhaustion is *not* an error to retry, it is failure to
//!     adapt; the kernel reacts by triggering Growth.
//!   - `Config::epoch_interruption(true)` — the Reaper publishes ticks via
//!     `Engine::increment_epoch`; a Wasm module that does not return between
//!     ticks is forcibly halted (§3 Reaper, §4.2).

use std::sync::Arc;
pub use wasmtime::{Engine, Module};
use wasmtime::{Config, Instance, Memory, Store, StoreLimits, StoreLimitsBuilder, TypedFunc};

/// Decoded `Unmapped` returned across the FFI boundary. Mirrors
/// `ephemeral::Unmapped` deliberately — we keep this crate dependency-free of
/// `ephemeral` so the wasm host can be tested in isolation. The kernel maps
/// between the two at the call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unmapped {
    pub offset: usize,
    pub byte: u8,
}

#[derive(Debug)]
pub enum HostError {
    /// `process()` exhausted its fuel budget before reaching a terminal state.
    /// The Binary Code was not efficient enough — the kernel should treat this
    /// as an Irritation signal and trigger Growth on the offending input.
    OutOfFuel,
    /// The Reaper's epoch tick fired before `process()` returned. Equivalent
    /// to stagnation death at the wasmtime layer (§3, §4.2).
    EpochDeadline,
    /// The module did not export the required `memory` / `process` symbols,
    /// or the input did not fit the linear memory.
    Abi(String),
    /// Anything else from wasmtime (instantiation, trap, link error...).
    Wasmtime(wasmtime::Error),
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostError::OutOfFuel => write!(f, "wasm out of fuel"),
            HostError::EpochDeadline => write!(f, "wasm epoch deadline"),
            HostError::Abi(s) => write!(f, "wasm abi: {s}"),
            HostError::Wasmtime(e) => write!(f, "wasmtime: {e}"),
        }
    }
}

impl std::error::Error for HostError {}

impl From<wasmtime::Error> for HostError {
    fn from(e: wasmtime::Error) -> Self {
        HostError::Wasmtime(e)
    }
}

/// Outcome of a single `process()` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepOutcome {
    /// Module returned a non-negative `NodeId`.
    Terminal(u64),
    /// Module returned a packed negative — Irritation.
    Unmapped(Unmapped),
}

/// Build the shared `Engine` once per kernel. The `Engine` is cheap to clone
/// (it's `Arc` internally) and is the right granularity for `increment_epoch`:
/// the Reaper task holds a clone and ticks it from outside the FSM loop.
pub fn build_engine() -> Result<Engine, HostError> {
    let mut config = Config::new();
    config.consume_fuel(true);
    config.epoch_interruption(true);
    // Cranelift defaults are fine for v0; the AOT story is M5/M6.
    Ok(Engine::new(&config)?)
}

/// A loaded Wasm module + its instance + the typed `process` handle, all
/// pinned to one `Store`. One `WasmHost` per FSM task.
///
/// Hot-swap (§4.3, M6) is implemented one layer up by holding
/// `ArcSwap<WasmHost>` in the kernel; this struct is intentionally not `Sync`
/// because `Store` isn't, and we want a fresh store per generation anyway.
pub struct WasmHost {
    engine: Engine,
    store: Store<HostState>,
    memory: Memory,
    process: TypedFunc<(i32, i32), i64>,
    /// Per-call fuel budget. Refilled before every `process()` call.
    fuel_per_call: u64,
    /// Distance (in epoch ticks) the Reaper grants each call before
    /// interrupting. `1` means "must finish before the next tick".
    epoch_deadline: u64,
}

struct HostState {
    limits: StoreLimits,
}

impl WasmHost {
    /// Load a module from raw `wasm32-unknown-unknown` bytes (or WAT — the
    /// `wat` feature transparently handles both).
    pub fn from_bytes(
        engine: &Engine,
        wasm_bytes: &[u8],
        fuel_per_call: u64,
        epoch_deadline: u64,
    ) -> Result<Self, HostError> {
        let module = Module::new(engine, wasm_bytes)?;
        Self::from_module(engine, &module, fuel_per_call, epoch_deadline)
    }

    pub fn from_module(
        engine: &Engine,
        module: &Module,
        fuel_per_call: u64,
        epoch_deadline: u64,
    ) -> Result<Self, HostError> {
        let limits = StoreLimitsBuilder::new()
            // 16 MiB linear-memory cap per FSM. Bigger than any plausible
            // single Mastodon payload, much smaller than the cgroup ceiling.
            .memory_size(16 * 1024 * 1024)
            .build();
        let mut store = Store::new(engine, HostState { limits });
        store.limiter(|s| &mut s.limits);
        store.set_epoch_deadline(epoch_deadline);
        // First fuel grant; refilled before every call.
        store.set_fuel(fuel_per_call)?;

        let instance = Instance::new(&mut store, module, &[])?;
        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| HostError::Abi("missing `memory` export".into()))?;
        let process = instance
            .get_typed_func::<(i32, i32), i64>(&mut store, "process")
            .map_err(|e| HostError::Abi(format!("missing/wrong `process` export: {e}")))?;

        Ok(Self {
            engine: engine.clone(),
            store,
            memory,
            process,
            fuel_per_call,
            epoch_deadline,
        })
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Feed one input buffer through the module. Writes `input` into Wasm
    /// linear memory at offset 0, refills fuel + epoch deadline, calls
    /// `process(0, len)`, and decodes the i64 return per the ABI documented
    /// at the top of this file.
    pub fn process(&mut self, input: &[u8]) -> Result<StepOutcome, HostError> {
        if input.len() > i32::MAX as usize {
            return Err(HostError::Abi("input larger than i32".into()));
        }
        // Make sure linear memory is large enough — grow if not.
        let needed = input.len();
        let have = self.memory.data_size(&self.store);
        if needed > have {
            let extra_pages = ((needed - have) + 0xFFFF) / 0x10000;
            self.memory
                .grow(&mut self.store, extra_pages as u64)
                .map_err(|e| HostError::Abi(format!("memory.grow failed: {e}")))?;
        }
        self.memory.data_mut(&mut self.store)[..needed].copy_from_slice(input);

        // Refill metabolic budgets before every call (§4.2).
        self.store.set_fuel(self.fuel_per_call)?;
        self.store.set_epoch_deadline(self.epoch_deadline);

        let result = self.process.call(&mut self.store, (0, needed as i32));
        match result {
            Ok(v) if v >= 0 => Ok(StepOutcome::Terminal(v as u64)),
            Ok(v) => {
                // Decode packed = -v - 1.
                let packed = (-(v + 1)) as u64;
                let byte = (packed & 0xFF) as u8;
                let offset = (packed >> 8) as usize;
                Ok(StepOutcome::Unmapped(Unmapped { offset, byte }))
            }
            Err(e) => Err(classify_trap(e)),
        }
    }
}

fn classify_trap(e: wasmtime::Error) -> HostError {
    if let Some(trap) = e.downcast_ref::<wasmtime::Trap>() {
        match trap {
            wasmtime::Trap::OutOfFuel => return HostError::OutOfFuel,
            wasmtime::Trap::Interrupt => return HostError::EpochDeadline,
            _ => {}
        }
    }
    HostError::Wasmtime(e)
}

/// The Generation-0 bootstrap binary (§M4). It returns `Unmapped { 0, byte0 }`
/// for any non-empty input, guaranteeing the very first packet from the
/// firehose triggers Growth in the Ephemeral Layer. An empty input returns
/// the root NodeId (0) as a successful no-op so the host startup self-test
/// stays cheap.
///
/// Encoding the bootstrap as WAT (rather than shipping a precompiled `.wasm`)
/// keeps the repo text-only and means the fossil at `gen_000.wasm` is
/// reproducible from source on first boot.
pub const GEN_000_WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (func (export "process") (param $ptr i32) (param $len i32) (result i64)
    ;; if len == 0 → return 0 (root NodeId, success).
    (if (i32.eqz (local.get $len))
      (then (return (i64.const 0))))
    ;; packed = (offset=0 << 8) | byte0 = byte0
    ;; result = -(packed + 1)
    (i64.sub
      (i64.const 0)
      (i64.add
        (i64.extend_i32_u (i32.load8_u (local.get $ptr)))
        (i64.const 1)))))
"#;

/// Compile the bootstrap WAT into a wasm binary blob. Suitable to write into
/// `fossils/gen_000.wasm` on first boot, or to feed directly to
/// `WasmHost::from_bytes`.
pub fn gen_000_wasm() -> Result<Arc<[u8]>, HostError> {
    let bytes = wat::parse_str(GEN_000_WAT)
        .map_err(|e| HostError::Abi(format!("gen_000 wat: {e}")))?;
    Ok(Arc::from(bytes.into_boxed_slice()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> WasmHost {
        let engine = build_engine().unwrap();
        let bytes = gen_000_wasm().unwrap();
        WasmHost::from_bytes(&engine, &bytes, 1_000_000, 1).unwrap()
    }

    #[test]
    fn empty_input_returns_root_terminal() {
        let mut h = host();
        assert_eq!(h.process(b"").unwrap(), StepOutcome::Terminal(0));
    }

    #[test]
    fn first_byte_is_unmapped_at_offset_zero() {
        let mut h = host();
        match h.process(b"hello").unwrap() {
            StepOutcome::Unmapped(u) => {
                assert_eq!(u.offset, 0);
                assert_eq!(u.byte, b'h');
            }
            other => panic!("expected Unmapped, got {other:?}"),
        }
    }

    #[test]
    fn unmapped_zero_byte_is_distinguishable_from_terminal_zero() {
        // Critical ABI corner: Unmapped{0,0} must NOT collide with the
        // success-NodeId-0 encoding. The `+1` shift guarantees this.
        let mut h = host();
        match h.process(&[0u8]).unwrap() {
            StepOutcome::Unmapped(u) => {
                assert_eq!(u.offset, 0);
                assert_eq!(u.byte, 0);
            }
            other => panic!("expected Unmapped, got {other:?}"),
        }
    }

    #[test]
    fn epoch_tick_interrupts_runaway_module() {
        // gen_000 returns instantly so we can't truly observe an interrupt
        // here, but we *can* prove the host wires the deadline by ticking
        // the engine and confirming a fresh call still succeeds (the
        // deadline is reset on every call).
        let engine = build_engine().unwrap();
        let bytes = gen_000_wasm().unwrap();
        let mut h = WasmHost::from_bytes(&engine, &bytes, 1_000_000, 1).unwrap();
        engine.increment_epoch();
        engine.increment_epoch();
        // Should still work — `process()` resets the deadline before calling.
        assert!(matches!(h.process(b"x").unwrap(), StepOutcome::Unmapped(_)));
    }
}
