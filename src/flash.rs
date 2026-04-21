//! ---------------------------------------------------------------------------
//! FLASH DRIVER (STM32C031 - MINIMAL SAFE HARDWARE LAYER)
//! ---------------------------------------------------------------------------
//!
//! Purpose:
//! - Provide correct erase/program primitives for EEPROM emulation
//! - No policy, no buffering, no state tracking
//! - Deterministic STM32C0 Flash sequencing
//!
//! ---------------------------------------------------------------------------

use stm32c0::stm32c031 as pac;

const FLASH_BASE: u32 = 0x0800_0000;
const PAGE_SIZE: u32 = 2048;

const FLASH_KEY1: u32 = 0x4567_0123;
const FLASH_KEY2: u32 = 0xCDEF_89AB;

/// ---------------------------------------------------------------------------
/// HARDWARE ERROR MODEL
/// ---------------------------------------------------------------------------
#[derive(Debug)]
pub enum FlashError {
    BusyTimeout,
    ProgramError,
    WriteProtect,
    AlignmentError,
}

/// ---------------------------------------------------------------------------
/// FLASH DRIVER
/// ---------------------------------------------------------------------------
pub struct Stm32Flash {
    flash: pac::FLASH,
}

impl Stm32Flash {
    /// Create + unlock Flash interface
    pub fn new(flash: pac::FLASH) -> Self {
        let mut f = Self { flash };
        f.unlock();
        f
    }

    /// Unlock Flash (always executed, no conditional logic)
    fn unlock(&mut self) {
        self.flash.keyr().write(|w| unsafe { w.bits(FLASH_KEY1) });
        self.flash.keyr().write(|w| unsafe { w.bits(FLASH_KEY2) });
    }

    /// Optional safety lock
    #[allow(dead_code)]
    fn lock(&mut self) {
        self.flash.cr().modify(|_, w| w.lock().set_bit());
    }

    /// Wait for Flash ready with error detection
    fn wait_ready(&self) -> Result<(), FlashError> {
        let mut timeout = 1_000_000;

        loop {
            let sr = self.flash.sr().read();

            if sr.progerr().bit_is_set() {
                self.clear_errors();
                return Err(FlashError::ProgramError);
            }

            if sr.wrperr().bit_is_set() {
                self.clear_errors();
                return Err(FlashError::WriteProtect);
            }

            if !sr.bsy1().bit_is_set() {
                return Ok(());
            }

            timeout -= 1;
            if timeout == 0 {
                return Err(FlashError::BusyTimeout);
            }
        }
    }

    /// Clear all error flags (1-to-clear semantics)
    fn clear_errors(&self) {
        self.flash.sr().modify(|_, w| {
            w.progerr().set_bit();
            w.wrperr().set_bit();
            w.pgaerr().set_bit();
            w.sizerr().set_bit();
            w.operr().set_bit()
        });
    }

    /// -----------------------------------------------------------------------
    /// ERASE PAGE
    /// -----------------------------------------------------------------------
    #[inline(never)]
    #[link_section = ".ramfunc"]
    pub fn erase_page(&mut self, page_addr: u32) -> Result<(), FlashError> {
        if page_addr < FLASH_BASE || page_addr % PAGE_SIZE != 0 {
            return Err(FlashError::AlignmentError);
        }

        self.wait_ready()?;

        let page = (page_addr - FLASH_BASE) / PAGE_SIZE;

        self.flash.cr().modify(|_, w| {
            w.mer1().clear_bit();
            unsafe { w.pnb().bits(page as u8) };
            w.per().set_bit()
        });

        self.flash.cr().modify(|_, w| w.strt().set_bit());

        self.wait_ready()?;

        self.flash.cr().modify(|_, w| w.per().clear_bit());

        Ok(())
    }

    /// -----------------------------------------------------------------------
    /// PROGRAM 64-BIT
    /// -----------------------------------------------------------------------
    #[inline(never)]
    #[link_section = ".ramfunc"]
    pub fn write_double_word(&mut self, addr: u32, data: u64) -> Result<(), FlashError> {
        if addr % 8 != 0 {
            return Err(FlashError::AlignmentError);
        }

        self.wait_ready()?;

        self.flash.cr().modify(|_, w| w.pg().set_bit());

        unsafe {
            core::ptr::write_volatile(addr as *mut u64, data);
        }

        self.wait_ready()?;

        self.flash.cr().modify(|_, w| w.pg().clear_bit());

        Ok(())
    }

    /// Read 32-bit
    #[inline(always)]
    pub fn read_word(&self, addr: u32) -> u32 {
        unsafe { core::ptr::read_volatile(addr as *const u32) }
    }

    /// Read 64-bit safe split
    #[inline(always)]
    pub fn read64(&self, addr: u32) -> u64 {
        let low = self.read_word(addr) as u64;
        let high = self.read_word(addr + 4) as u64;
        (high << 32) | low
    }

    /// Check erased state
    #[inline(always)]
    pub fn is_erased(&self, addr: u32) -> bool {
        self.read64(addr) == u64::MAX
    }
}