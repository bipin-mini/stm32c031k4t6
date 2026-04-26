#![allow(dead_code)]

use stm32c0::stm32c031::{EXTI, GPIOA};

/// ---------------------------------------------------------------------------
/// Quadrature Lookup Table (LUT)
/// ---------------------------------------------------------------------------
///
/// Encodes all valid transitions between previous and current AB states.
///
/// Index:
/// - index = (prev_state << 2) | curr_state
///
/// Bit layout:
/// - bit0 = A (PA0)
/// - bit1 = B (PA1)
///
/// Output:
/// - +1 → forward step
/// - -1 → reverse step
/// -  0 → no movement or invalid transition (ignored)
///
/// ---------------------------------------------------------------------------
/// Design Property
/// ---------------------------------------------------------------------------
///
/// The LUT fully replaces conditional logic:
/// - No branching
/// - Constant-time decoding
/// - Deterministic execution on Cortex-M0+
const LUT: [i8; 16] = [0, 1, -1, 0, -1, 0, 0, 1, 1, 0, 0, -1, 0, -1, 1, 0];

/// ---------------------------------------------------------------------------
/// ISR STATE
/// ---------------------------------------------------------------------------
///
/// Stored in static memory for zero-stack ISR execution.
static mut PREV_STATE: u8 = 0;
static mut COUNT: i32 = 0;

/// ---------------------------------------------------------------------------
/// HOTPATH POINTERS (FIXED FOR STM32C0 PAC v0.16.0)
/// ---------------------------------------------------------------------------
///
/// These pointers reference the actual register blocks.
/// No casting between PAC wrapper types is performed.
///
/// This avoids:
/// - E0606 invalid casts
/// - E0308 mismatched peripheral types
///
/// Performance goal:
/// - Eliminate repeated ptr() resolution inside ISR
type GpioaRb = stm32c0::stm32c031::gpioa::RegisterBlock;
type ExtiRb = stm32c0::stm32c031::exti::RegisterBlock;

static mut GPIOA_REF: *const GpioaRb = core::ptr::null();
static mut EXTI_REF: *const ExtiRb = core::ptr::null();

/// ---------------------------------------------------------------------------
/// Initialization
/// ---------------------------------------------------------------------------
///
/// Must be called before enabling interrupts.
pub fn init() {
    unsafe {
        let gpioa = &*GPIOA::ptr();

        // Store raw register block pointers
        GPIOA_REF = GPIOA::ptr();
        EXTI_REF = EXTI::ptr();

        let idr = gpioa.idr().read().bits();

        PREV_STATE = ((idr & 1) | (((idr >> 1) & 1) << 1)) as u8;
    }
}

/// ---------------------------------------------------------------------------
/// Public API
/// ---------------------------------------------------------------------------
#[inline(always)]
pub fn get_count() -> i32 {
    unsafe { COUNT }
}

#[inline(always)]
pub fn reset_count() {
    unsafe {
        COUNT = 0;
    }
}

/// ---------------------------------------------------------------------------
/// EXTI ISR (Cycle-optimized hot path)
/// ---------------------------------------------------------------------------
///
/// Design:
/// - No branching
/// - LUT-based decoding
/// - Minimal memory accesses
/// - Constant-time execution path
///
/// Expected Cortex-M0+ cost:
/// - ~45–60 cycles depending on flash wait states
#[inline(always)]
pub fn isr() {
    unsafe {
        let gpioa = &*GPIOA_REF;
        let exti = &*EXTI_REF;

        // Atomic GPIO snapshot
        let idr = gpioa.idr().read().bits();

        // Encode AB state
        let curr_state = (idr & 0x3) as u8;

        // LUT decode
        let index = ((PREV_STATE << 2) | curr_state) as usize;
        let delta = *LUT.get_unchecked(index);

        // Update counter
        COUNT = COUNT.wrapping_add(delta as i32);

        // Update state machine
        PREV_STATE = curr_state;

        // Clear EXTI flags (rising + falling)
        const MASK: u32 = (1 << 0) | (1 << 1);

        exti.rpr1().write(|w| w.bits(MASK));
        exti.fpr1().write(|w| w.bits(MASK));
    }
}
