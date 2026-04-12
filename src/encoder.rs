//! encoder.rs
//!
//! High-speed quadrature encoder decoder using EXTI interrupts.
//!
//! ---------------------------------------------------------------------------
//! 🧠 DESIGN INTENT
//! ---------------------------------------------------------------------------
//!
//! This module implements a **deterministic, interrupt-driven quadrature decoder**
//! capable of handling ≥100k edges/sec using software decoding.
//!
//! The implementation is specifically tuned for:
//! - Cortex-M0+ (no advanced pipeline, predictable timing)
//! - Zero HAL overhead (direct PAC register access)
//! - Constant-time ISR execution (cycle-invariant)
//!
//! ---------------------------------------------------------------------------
//! ⚙️ CORE DESIGN PRINCIPLES
//! ---------------------------------------------------------------------------
//!
//! 1. **Branchless decoding**
//!    - Eliminates timing variability
//!    - Uses LUT (lookup table) instead of conditional logic
//!
//! 2. **Atomic signal sampling**
//!    - Both encoder channels (A & B) are read from a single GPIO IDR read
//!    - Prevents race conditions between edges
//!
//! 3. **EXTI-agnostic ISR**
//!    - ISR does NOT depend on which EXTI line triggered
//!    - Always samples full encoder state
//!
//! 4. **Constant execution time**
//!    - No data-dependent branching in critical path
//!    - Ensures predictable interrupt latency
//!
//! 5. **Minimal ISR workload**
//!    - Only counting logic inside ISR
//!    - No scaling, UI, communication, or blocking operations
//!
//! ---------------------------------------------------------------------------
//! 🎯 PERFORMANCE TARGETS
//! ---------------------------------------------------------------------------
//!
//! - Encoder rate:          ≥ 100,000 edges/sec (X4 decoding)
//! - ISR execution:         ≤ 80 cycles (target ~50–60 cycles)
//! - Interrupt latency:     ≤ 2 µs
//!
//! ---------------------------------------------------------------------------
//! ⚠️ HARDWARE ASSUMPTIONS
//! ---------------------------------------------------------------------------
//!
//! - ENC_A → PA0 (EXTI0)
//! - ENC_B → PA1 (EXTI1)
//! - Both pins are on SAME GPIO port (GPIOA)
//! - EXTI configured for BOTH edges (rising + falling)
//!
//! ---------------------------------------------------------------------------
//! 🧩 MCU-SPECIFIC NOTE (STM32C0)
//! ---------------------------------------------------------------------------
//!
//! Unlike older STM32 families:
//!
//! - Rising pending flags  → RPR1
//! - Falling pending flags → FPR1
//!
//! BOTH must be cleared explicitly.
//!
//! ---------------------------------------------------------------------------

use stm32c0::stm32c031::{EXTI, GPIOA};

/// ---------------------------------------------------------------------------
/// 🔁 Quadrature Lookup Table (LUT)
/// ---------------------------------------------------------------------------
///
/// This table encodes all possible transitions between previous and current
/// encoder states.
///
/// Index construction:
///
///     index = (prev_state << 2) | curr_state
///
/// Where:
///     prev_state = previous AB (2 bits)
///     curr_state = current AB (2 bits)
///
/// Bit encoding:
///     bit0 = A (PA0)
///     bit1 = B (PA1)
///
/// Example:
///     prev = 01 (A=1, B=0)
///     curr = 11 (A=1, B=1)
///
///     index = 0b0111 = 7
///
/// Output meaning:
///     +1 → forward step
///     -1 → reverse step
///      0 → no movement OR invalid transition
///
/// ---------------------------------------------------------------------------
/// 🧠 Why LUT?
/// ---------------------------------------------------------------------------
///
/// - Eliminates conditional branching
/// - Guarantees constant execution time
/// - Encodes full state machine in data
///
/// ---------------------------------------------------------------------------
/// ⚠️ Invalid transitions
/// ---------------------------------------------------------------------------
///
/// These occur when:
/// - edges are missed (ISR too slow)
/// - signal integrity issues (noise, bounce)
///
/// Example:
///     00 → 11 (both bits change simultaneously)
///
/// These are mapped to 0 (ignored)
///
#[link_section = ".data"]
static LUT: [i8; 16] = [
    // prev = 00
     0,  1, -1,  0,
    // prev = 01
    -1,  0,  0,  1,
    // prev = 10
     1,  0,  0, -1,
    // prev = 11
     0, -1,  1,  0,
];

/// ---------------------------------------------------------------------------
/// 🔒 Internal State (ISR-owned)
/// ---------------------------------------------------------------------------
///
/// These variables are ONLY modified inside ISR.
///
/// Safety rationale:
/// - 32-bit reads/writes are atomic on Cortex-M0+
/// - No concurrent writers exist
/// - Main loop only performs non-atomic reads (acceptable here)
///

/// Previous encoder state (2-bit: AB)
///
/// Stores last sampled state to form transition index
static mut PREV_STATE: u8 = 0;

/// 32-bit signed pulse counter
///
/// Wraparound behavior is intentional and defined
static mut COUNT: i32 = 0;

/// Counts invalid transitions
///
/// Useful for:
/// - diagnostics
/// - detecting missed edges
/// - EMI/noise analysis
static mut INVALID_COUNT: u32 = 0;

/// ---------------------------------------------------------------------------
/// 🔧 Initialization
/// ---------------------------------------------------------------------------
///
/// Must be called AFTER GPIO configuration and BEFORE enabling interrupts.
///
/// ---------------------------------------------------------------------------
/// 🧠 Why needed?
/// ---------------------------------------------------------------------------
///
/// Without initialization:
/// - First interrupt would compare against undefined PREV_STATE
/// - Could produce false count (±1 error)
///
pub fn init() {
    unsafe {
        let gpioa = &*GPIOA::ptr();

        // Read current GPIO state (atomic snapshot)
        let idr = gpioa.idr().read().bits();

        // Extract initial A/B state
        PREV_STATE = (
            ((idr >> 0) & 1) |        // A → bit0
            (((idr >> 1) & 1) << 1)   // B → bit1
        ) as u8;
    }
}

/// ---------------------------------------------------------------------------
/// 📤 Public API (Main Loop Access)
/// ---------------------------------------------------------------------------

/// Returns current encoder count
///
/// ---------------------------------------------------------------------------
/// 🧠 Concurrency note:
/// ---------------------------------------------------------------------------
///
/// - COUNT is updated in ISR
/// - This read is NOT synchronized
///
/// However:
/// - 32-bit access is atomic on Cortex-M0+
/// - No partial reads possible
///
/// Result:
/// - Safe for real-time systems
/// - Value may be slightly stale, but never corrupted
///
#[inline(always)]
pub fn get_count() -> i32 {
    unsafe { COUNT }
}

/// Resets encoder count to zero
///
/// ---------------------------------------------------------------------------
/// ⚠️ Note:
/// ---------------------------------------------------------------------------
///
/// If strict consistency is required:
/// - disable interrupts before calling
///
#[inline(always)]
pub fn reset_count() {
    unsafe {
        COUNT = 0;
    }
}

/// Returns number of invalid transitions detected
///
/// Useful for:
/// - system diagnostics
/// - validating signal integrity
///
#[inline(always)]
pub fn get_invalid_count() -> u32 {
    unsafe { INVALID_COUNT }
}

/// ---------------------------------------------------------------------------
/// ⚡ EXTI Interrupt Handler (Core Logic)
/// ---------------------------------------------------------------------------
///
/// Must be called from:
///     EXTI0_1 interrupt vector
///
/// ---------------------------------------------------------------------------
/// 🧠 Execution Model
/// ---------------------------------------------------------------------------
///
/// This ISR:
/// 1. Reads both encoder inputs atomically
/// 2. Computes transition index
/// 3. Looks up delta from LUT
/// 4. Updates counter
/// 5. Clears interrupt flags
///
/// ---------------------------------------------------------------------------
/// ⚠️ CRITICAL RULES
/// ---------------------------------------------------------------------------
///
/// - MUST NOT branch based on EXTI source
/// - MUST read BOTH A and B every time
/// - MUST clear BOTH EXTI flags
/// - MUST execute in constant time
///
#[inline(always)]
pub fn isr() {
    unsafe {
        // -----------------------------------------------------------------
        // 1. Direct peripheral access (constant addresses, optimized by compiler)
        // -----------------------------------------------------------------
        let gpioa = &*GPIOA::ptr();
        let exti  = &*EXTI::ptr();

        // -----------------------------------------------------------------
        // 2. Atomic GPIO read
        // -----------------------------------------------------------------
        let idr = gpioa.idr().read().bits();

        // -----------------------------------------------------------------
        // 3. Extract A/B in ONE instruction
        // -----------------------------------------------------------------
        // PA0 -> bit0, PA1 -> bit1
        let curr_state = (idr & 0x3) as u8;

        // -----------------------------------------------------------------
        // 4. Build LUT index (2-bit prev + 2-bit current)
        // -----------------------------------------------------------------
        let prev = PREV_STATE;
        let index = ((prev << 2) | curr_state) as usize;

        // -----------------------------------------------------------------
        // 5. LUT decode (no bounds check)
        // -----------------------------------------------------------------
        let delta = *LUT.get_unchecked(index);

        // -----------------------------------------------------------------
        // 6. Update counter (single load/add/store)
        // -----------------------------------------------------------------
        COUNT = COUNT.wrapping_add(delta as i32);

        // -----------------------------------------------------------------
        // 7. Optional: INVALID detection (branchless version)
        // -----------------------------------------------------------------
        // Condition:
        // delta == 0 AND state changed
        //
        // Convert boolean -> 0/1 and accumulate
        let invalid = ((delta == 0) as u32) & ((prev != curr_state) as u32);
        INVALID_COUNT = INVALID_COUNT.wrapping_add(invalid);

        // -----------------------------------------------------------------
        // 8. Update previous state
        // -----------------------------------------------------------------
        PREV_STATE = curr_state;

        // -----------------------------------------------------------------
        // 9. Clear EXTI flags (RAW WRITE - fastest possible)
        // -----------------------------------------------------------------
        // Clear EXTI0 + EXTI1 for BOTH rising and falling
        const MASK: u32 = (1 << 0) | (1 << 1);

        exti.rpr1().write(|w| unsafe { w.bits(MASK) });
        exti.fpr1().write(|w| unsafe { w.bits(MASK) });
    }
}