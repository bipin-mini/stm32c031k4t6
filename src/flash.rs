//! ---------------------------------------------------------------------------
//! FLASH DRIVER (STM32C031 / stm32c0 v0.16)
//! ---------------------------------------------------------------------------
//!
//! Low-level Flash memory access layer for STM32C0 series devices.
//!
//! ---------------------------------------------------------------------------
//! 🧠 PURPOSE
//! ---------------------------------------------------------------------------
//!
//! Provides a minimal, blocking API for:
//!
//! - Page erase (2 KB granularity)
//! - 64-bit programming (double-word)
//! - 32-bit read access
//!
//! This module is intentionally:
//!
//! - Stateless (no internal state machine)
//! - Deterministic (bounded blocking operations)
//! - Hardware-focused (no policy, no wear leveling)
//!
//! It is designed to be used by higher-level storage layers such as
//! EEPROM emulation compliant with ST AN4894.
//!
//! ---------------------------------------------------------------------------
//! ⚠️ HARDWARE CONSTRAINTS (STM32C0)
//! ---------------------------------------------------------------------------
//!
//! - Flash is **single-bank**
//! - CPU stalls during program/erase operations if executing from Flash
//! - Programming granularity: **64-bit (double word)**
//! - Writes MUST be **8-byte aligned**
//! - Flash can only change bits **1 → 0**
//!   → erase required to restore bits to 1
//!
//! CONTROL FLOW REQUIREMENTS:
//!
//! - BSY flag MUST be polled before/after operations
//! - Error flags are **sticky** and MUST be cleared manually
//!
//! ---------------------------------------------------------------------------
//! ⚠️ EXECUTION MODEL (IMPORTANT)
//! ---------------------------------------------------------------------------
//!
//! This implementation executes from Flash.
//!
//! EFFECT:
//! - During Flash operations, CPU will stall (bus wait)
//! - This is acceptable on STM32C0 due to simple single-bank design
//!
//! NOTE:
//! - `.ramfunc` execution is NOT enforced here
//! - If strict real-time behavior is required, caller must:
//!     - Move functions to RAM
//!     - Ensure all dependencies are also in RAM
//!
//! ---------------------------------------------------------------------------
//! ⚠️ INTERRUPT SAFETY CONTRACT
//! ---------------------------------------------------------------------------
//!
//! This driver is:
//!
//! - Blocking
//! - NOT interrupt-safe
//!
//! CALLER MUST ENSURE:
//!
//! - Interrupts are disabled during:
//!     - erase_page()
//!     - write_double_word()
//!
//! FAILURE TO DO SO MAY RESULT IN:
//!
//! - ISR execution stalling (Flash not readable during BSY)
//! - Timing violations in real-time system
//! - Undefined system behavior under load
//!
//! ---------------------------------------------------------------------------
//! ⚠️ POWER-FAIL SAFETY CONTRACT
//! ---------------------------------------------------------------------------
//!
//! This driver provides **NO guarantees** under power loss.
//!
//! SYSTEM MUST PROVIDE:
//!
//! - Early power-fail detection (EXTI or analog comparator)
//! - Sufficient hold-up time (external capacitor)
//! - Controlled shutdown sequence
//!
//! RULES:
//!
//! - erase_page() MUST NOT be used during power-fail handling
//! - write_double_word() MAY be used if:
//!     - operation is bounded
//!     - supply remains stable for full duration
//!
//! ---------------------------------------------------------------------------

#[allow(dead_code)]
use stm32c0::stm32c031 as pac;

/// Flash operation errors
///
/// ---------------------------------------------------------------------------
/// 🧠 ERROR MODEL
/// ---------------------------------------------------------------------------
///
/// These errors represent **hardware-level failures**:
///
/// - BusyTimeout  → Flash did not become ready
/// - ProgramError → Programming sequence failed
/// - WriteProtect → Region is write-protected
///
/// Higher layers (EEPROM) must handle:
/// - Retry policies
/// - Data integrity
/// - Recovery strategies
///
#[derive(Debug)]
pub enum FlashError {
    /// Flash remained busy beyond timeout
    BusyTimeout,

    /// Programming error (alignment, invalid sequence, verify failure, etc.)
    ProgramError,

    /// Write protection error
    WriteProtect,
}

/// Blocking Flash driver
///
/// ---------------------------------------------------------------------------
/// 🧠 DESIGN MODEL
/// ---------------------------------------------------------------------------
///
/// - Owns FLASH peripheral (exclusive access)
/// - Provides synchronous operations
/// - No buffering, no caching
///
/// This layer is intentionally minimal.
/// All higher-level logic must be implemented above it.
///
pub struct Stm32Flash {
    flash: pac::FLASH,
}

impl Stm32Flash {
    /// Create new Flash driver
    ///
    /// -----------------------------------------------------------------------
    /// ⚠️ SAFETY CONTRACT
    /// -----------------------------------------------------------------------
    ///
    /// - Must be called exactly once
    /// - Caller guarantees exclusive ownership of FLASH peripheral
    ///
    pub fn new(flash: pac::FLASH) -> Self {
        Self { flash }
    }

    /// Wait until Flash is ready (BSY = 0)
    ///
    /// -----------------------------------------------------------------------
    /// 🧠 BEHAVIOR
    /// -----------------------------------------------------------------------
    ///
    /// - Polls BSY1 flag (STM32C0 specific)
    /// - Uses simple decrementing timeout
    ///
    /// -----------------------------------------------------------------------
    /// ⚠️ LIMITATIONS
    /// -----------------------------------------------------------------------
    ///
    /// - Timeout is CPU-cycle based (not time-accurate)
    /// - Depends on SYSCLK frequency
    /// - Must be sized conservatively for worst-case conditions
    ///
    /// -----------------------------------------------------------------------
    #[inline(always)]
    fn wait_ready(&self) -> Result<(), FlashError> {
        let mut timeout = 1_000_000;

        while self.flash.sr().read().bsy1().bit_is_set() {
            timeout -= 1;
            if timeout == 0 {
                return Err(FlashError::BusyTimeout);
            }
        }

        Ok(())
    }

    /// Check and clear Flash error flags
    ///
    /// -----------------------------------------------------------------------
    /// 🧠 BEHAVIOR
    /// -----------------------------------------------------------------------
    ///
    /// - Reads SR register
    /// - Detects error conditions
    /// - Clears flags using write-1-to-clear semantics
    ///
    /// -----------------------------------------------------------------------
    /// ⚠️ REQUIREMENT
    /// -----------------------------------------------------------------------
    ///
    /// - Must be called after every operation
    /// - Ensures next operation starts from clean state
    ///
    /// -----------------------------------------------------------------------
    #[inline(always)]
    fn check_errors(&self) -> Result<(), FlashError> {
        let sr = self.flash.sr().read();

        if sr.progerr().bit_is_set() {
            self.flash.sr().modify(|_, w| w.progerr().set_bit());
            return Err(FlashError::ProgramError);
        }

        if sr.wrperr().bit_is_set() {
            self.flash.sr().modify(|_, w| w.wrperr().set_bit());
            return Err(FlashError::WriteProtect);
        }

        Ok(())
    }

    /// Erase one Flash page
    ///
    /// -----------------------------------------------------------------------
    /// 🧠 OPERATION
    /// -----------------------------------------------------------------------
    ///
    /// - Converts address → page index
    /// - Enables erase mode
    /// - Starts erase sequence
    /// - Waits for completion
    ///
    /// -----------------------------------------------------------------------
    /// ⚠️ REQUIREMENTS
    /// -----------------------------------------------------------------------
    ///
    /// - `page_address` MUST be 2 KB aligned
    /// - Interrupts MUST be disabled by caller
    /// - Page MUST belong to valid Flash region
    ///
    /// -----------------------------------------------------------------------
    /// ⚠️ TIMING
    /// -----------------------------------------------------------------------
    ///
    /// - Typical: ~20 ms
    /// - Blocking operation
    ///
    /// -----------------------------------------------------------------------
    /// ⚠️ POWER-FAIL
    /// -----------------------------------------------------------------------
    ///
    /// MUST NOT be used during power-fail handling.
    ///
    /// -----------------------------------------------------------------------
    #[inline(never)]
    #[link_section = ".ramfunc"]
    pub fn erase_page(&mut self, page_address: u32) -> Result<(), FlashError> {
        self.wait_ready()?;

        // Convert address → page index
        // STM32C0: base = 0x0800_0000, page = 2 KB
        let page = (page_address - 0x0800_0000) / 2048;

        self.flash
            .cr()
            .modify(|_, w| unsafe { w.pnb().bits(page as u8) });

        self.flash.cr().modify(|_, w| w.per().set_bit());
        self.flash.cr().modify(|_, w| w.strt().set_bit());

        self.wait_ready()?;
        self.check_errors()?;

        self.flash.cr().modify(|_, w| w.per().clear_bit());

        Ok(())
    }

    /// Program one 64-bit double word
    ///
    /// -----------------------------------------------------------------------
    /// 🧠 OPERATION
    /// -----------------------------------------------------------------------
    ///
    /// - Enables programming mode
    /// - Performs atomic 64-bit write
    /// - Waits for completion
    /// - Checks for errors
    ///
    /// -----------------------------------------------------------------------
    /// ⚠️ REQUIREMENTS
    /// -----------------------------------------------------------------------
    ///
    /// - Address MUST be 8-byte aligned
    /// - Destination MUST be erased (all bits = 1)
    /// - Interrupts MUST be disabled by caller
    ///
    /// -----------------------------------------------------------------------
    /// ⚠️ TIMING
    /// -----------------------------------------------------------------------
    ///
    /// - Typical: ~40–80 µs
    /// - Bounded operation (safe for power-fail path if budget allows)
    ///
    /// -----------------------------------------------------------------------
    /// ⚠️ DATA INTEGRITY
    /// -----------------------------------------------------------------------
    ///
    /// - No internal verification is performed
    /// - Caller should verify write if required
    ///
    /// -----------------------------------------------------------------------
    #[inline(never)]
    #[link_section = ".ramfunc"]
    pub fn write_double_word(&mut self, address: u32, data: u64) -> Result<(), FlashError> {
        self.wait_ready()?;

        self.flash.cr().modify(|_, w| w.pg().set_bit());

        unsafe {
            core::ptr::write_volatile(address as *mut u64, data);
        }

        self.wait_ready()?;
        self.check_errors()?;

        self.flash.cr().modify(|_, w| w.pg().clear_bit());

        Ok(())
    }

    /// Read 32-bit word from Flash
    ///
    /// -----------------------------------------------------------------------
    /// 🧠 PROPERTIES
    /// -----------------------------------------------------------------------
    ///
    /// - Safe operation (no side effects)
    /// - Works during normal execution
    /// - Uses volatile access to prevent optimization
    ///
    #[inline(always)]
    pub fn read_word(&self, address: u32) -> u32 {
        unsafe { core::ptr::read_volatile(address as *const u32) }
    }
}