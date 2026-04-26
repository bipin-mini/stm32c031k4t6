//! ---------------------------------------------------------------------------
//! FLASH DRIVER (STM32C031 - HARDENED MINIMAL HARDWARE LAYER)
//! ---------------------------------------------------------------------------
//!
//! PURPOSE
//! ---------------------------------------------------------------------------
//! Provides deterministic, low-level Flash control primitives:
//! - Page erase
//! - 64-bit programming (double-word)
//! - Raw read helpers
//!
//! This driver is intentionally:
//! - Stateless
//! - Blocking
//! - Policy-free
//!
//! It is designed to be used as the foundation for EEPROM emulation
//! or other deterministic storage layers.
//!
//! ---------------------------------------------------------------------------
//! ⚠️ CRITICAL SYSTEM REQUIREMENTS (MUST READ)
//! ---------------------------------------------------------------------------
//!
//! ❗ INTERRUPTS MUST BE DISABLED DURING FLASH OPERATIONS
//!
//! This driver DOES NOT disable interrupts internally.
//!
//! On STM32C0 (Cortex-M0+):
//! - Flash cannot be read while programming/erase is ongoing
//! - Interrupt handlers execute from Flash
//!
//! If an interrupt occurs during Flash operation:
//! - CPU will attempt to fetch ISR from Flash
//! - Flash is busy → bus stall or HardFault
//!
//! ➤ REQUIRED USAGE:
//!
//! ```ignore
//! cortex_m::interrupt::free(|_| {
//!     flash.write_double_word(addr, data)?;
//! });
//! ```
//!
//! Applies to:
//! - erase_page()
//! - write_double_word()
//!
//! ❗ FAILURE TO DO THIS RESULTS IN UNDEFINED SYSTEM BEHAVIOR
//!
//! ---------------------------------------------------------------------------
//! ⚠️ EXECUTION LOCATION REQUIREMENT
//! ---------------------------------------------------------------------------
//!
//! Flash operations must execute from RAM:
//!
//! - Functions are placed in `.ramfunc`
//! - Linker script MUST copy them to RAM before execution
//!
//! Failure results in:
//! - CPU executing from Flash while Flash is busy → crash
//!
//! ---------------------------------------------------------------------------
//! HARDWARE ASSUMPTIONS (STM32C031)
//! ---------------------------------------------------------------------------
//!
//! - Page size: 2 KB
//! - Programming granularity: 64-bit (double word)
//! - Flash writes are blocking
//! - Bits only transition: 1 → 0
//! - Erase resets entire page to 0xFF
//!
//! ---------------------------------------------------------------------------

use core::sync::atomic::{Ordering, compiler_fence};
use stm32c0::stm32c031 as pac;

/// Flash base address
const FLASH_BASE: u32 = 0x0800_0000;

/// Page size (STM32C031)
const PAGE_SIZE: u32 = 2048;

/// Flash unlock keys
const FLASH_KEY1: u32 = 0x4567_0123;
const FLASH_KEY2: u32 = 0xCDEF_89AB;

/// ---------------------------------------------------------------------------
/// HARDWARE ERROR MODEL
/// ---------------------------------------------------------------------------
#[derive(Debug)]
pub enum FlashError {
    /// Flash remained busy beyond timeout
    BusyTimeout,

    /// Programming error (alignment, invalid sequence, etc.)
    ProgramError,

    /// Write protection active
    WriteProtect,

    /// Invalid address alignment
    AlignmentError,
}

/// ---------------------------------------------------------------------------
/// FLASH DRIVER (PURE HARDWARE CONTROL)
// ---------------------------------------------------------------------------
pub struct Stm32Flash {
    flash: pac::FLASH,
}

impl Stm32Flash {
    /// -----------------------------------------------------------------------
    /// CREATE DRIVER
    /// -----------------------------------------------------------------------
    ///
    /// Unlocks Flash interface if locked.
    ///
    /// Must be called once during system initialization.
    ///
    pub fn new(flash: pac::FLASH) -> Self {
        let mut f = Self { flash };
        f.unlock();
        f
    }

    // ================================================================
    // UNLOCK FLASH (SAFE)
    // ================================================================
    ///
    /// Unlock sequence required before any erase/program operation.
    ///
    /// Only executed if LOCK bit is set.
    ///
    fn unlock(&mut self) {
        if self.flash.cr().read().lock().bit_is_set() {
            self.flash.keyr().write(|w| unsafe { w.bits(FLASH_KEY1) });
            self.flash.keyr().write(|w| unsafe { w.bits(FLASH_KEY2) });
        }
    }

    // ================================================================
    // OPTIONAL LOCK
    // ================================================================
    ///
    /// Can be used to prevent accidental writes after operations.
    ///
    #[allow(dead_code)]
    fn lock(&mut self) {
        self.flash.cr().modify(|_, w| w.lock().set_bit());
    }

    // ================================================================
    // WAIT READY (BSY + ERROR + EOP)
    // ================================================================
    ///
    /// Behavior:
    /// - Polls BSY flag
    /// - Detects errors
    /// - Clears EOP (End Of Operation)
    ///
    /// Must be called:
    /// - Before starting an operation
    /// - After completing an operation
    ///
    fn wait_ready(&self) -> Result<(), FlashError> {
        let mut timeout = 1_000_000;

        loop {
            let sr = self.flash.sr().read();

            // --- error detection ---
            if sr.progerr().bit_is_set() {
                self.clear_errors();
                return Err(FlashError::ProgramError);
            }

            if sr.wrperr().bit_is_set() {
                self.clear_errors();
                return Err(FlashError::WriteProtect);
            }

            // --- ready ---
            if !sr.bsy1().bit_is_set() {
                // Clear EOP if set (required by hardware)
                if sr.eop().bit_is_set() {
                    self.flash.sr().modify(|_, w| w.eop().set_bit());
                }

                return Ok(());
            }

            // --- timeout ---
            timeout -= 1;
            if timeout == 0 {
                return Err(FlashError::BusyTimeout);
            }
        }
    }

    // ================================================================
    // CLEAR ERROR FLAGS
    // ================================================================
    ///
    /// Clears all relevant error flags.
    /// Must be called before starting new operations.
    ///
    fn clear_errors(&self) {
        self.flash.sr().modify(|_, w| {
            w.progerr().set_bit();
            w.wrperr().set_bit();
            w.pgaerr().set_bit();
            w.sizerr().set_bit();
            w.operr().set_bit();
            w.eop().set_bit()
        });
    }

    // ================================================================
    // ERASE PAGE
    // ================================================================
    ///
    /// ⚠️ REQUIREMENTS:
    /// - Address must be page-aligned
    /// - Must execute from RAM (.ramfunc)
    /// - Interrupts MUST be disabled by caller
    ///
    /// Operation is blocking.
    ///
    #[inline(never)]
    #[unsafe(link_section = ".ramfunc")]
    pub fn erase_page(&mut self, page_addr: u32) -> Result<(), FlashError> {
        if page_addr < FLASH_BASE || !page_addr.is_multiple_of(PAGE_SIZE) {
            return Err(FlashError::AlignmentError);
        }

        self.wait_ready()?;
        self.clear_errors();

        let page = (page_addr - FLASH_BASE) / PAGE_SIZE;

        // Configure page erase
        self.flash.cr().modify(|_, w| {
            w.mer1().clear_bit();
            unsafe { w.pnb().bits(page as u8) };
            w.per().set_bit()
        });

        // Start erase
        self.flash.cr().modify(|_, w| w.strt().set_bit());

        self.wait_ready()?;

        // Disable erase mode
        self.flash.cr().modify(|_, w| w.per().clear_bit());

        Ok(())
    }

    // ================================================================
    // PROGRAM DOUBLE WORD (64-BIT)
    // ================================================================
    ///
    /// ⚠️ REQUIREMENTS:
    /// - Address must be 8-byte aligned
    /// - Target location must be erased
    /// - Must execute from RAM (.ramfunc)
    /// - Interrupts MUST be disabled by caller
    ///
    #[inline(never)]
    #[unsafe(link_section = ".ramfunc")]
    pub fn write_double_word(&mut self, addr: u32, data: u64) -> Result<(), FlashError> {
        if !addr.is_multiple_of(8) {
            return Err(FlashError::AlignmentError);
        }

        self.wait_ready()?;
        self.clear_errors();

        // Enable programming
        self.flash.cr().modify(|_, w| w.pg().set_bit());

        // Ensure correct ordering before write
        compiler_fence(Ordering::SeqCst);

        // Perform 64-bit write
        unsafe {
            core::ptr::write_volatile(addr as *mut u64, data);
        }

        self.wait_ready()?;

        // Disable programming
        self.flash.cr().modify(|_, w| w.pg().clear_bit());

        Ok(())
    }

    // ================================================================
    // READ HELPERS
    // ================================================================
    ///
    /// Safe volatile reads from Flash memory
    ///
    #[inline(always)]
    pub fn read_word(&self, addr: u32) -> u32 {
        unsafe { core::ptr::read_volatile(addr as *const u32) }
    }

    #[inline(always)]
    pub fn read64(&self, addr: u32) -> u64 {
        let low = self.read_word(addr) as u64;
        let high = self.read_word(addr + 4) as u64;
        (high << 32) | low
    }

    #[inline(always)]
    pub fn is_erased(&self, addr: u32) -> bool {
        self.read64(addr) == u64::MAX
    }
}
