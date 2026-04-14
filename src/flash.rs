#![allow(dead_code)]

use core::ptr;
use stm32c0::stm32c031 as pac;

/// ---------------------------------------------------------------------------
/// 📌 GENERIC FLASH PERSISTENCE DRIVER
/// ---------------------------------------------------------------------------
///
/// This module provides a **minimal, deterministic flash storage layer**
/// for STM32C0 microcontrollers using raw PAC access.
///
/// ---------------------------------------------------------------------------
/// 🧠 DESIGN GOALS
/// ---------------------------------------------------------------------------
///
/// ✔ Store fixed-size binary blobs (`[u8; N]`)
/// ✔ CRC16 integrity protection (Modbus polynomial)
/// ✔ Single-page overwrite model (simple + reliable)
/// ✔ Fully deterministic (no allocation, no runtime state)
/// ✔ No ISR usage (safe for RTIC systems)
///
/// ---------------------------------------------------------------------------
/// ⚠️ HARDWARE MODEL (STM32C0)
/// ---------------------------------------------------------------------------
///
/// Flash behavior:
/// - Erase is page-based (NOT address-based)
/// - Programming is half-word (16-bit)
/// - BUSY flag is `bsy1`
/// - Unlock required before write
///
/// ---------------------------------------------------------------------------
/// 📍 MEMORY LAYOUT
/// ---------------------------------------------------------------------------
///
///     FLASH PAGE (reserved region)
///
///     +--------------------------+
///     | data: [u8; N]           |
///     | crc:  u16               |
///     +--------------------------+
///
/// ---------------------------------------------------------------------------
/// ⚠️ SYSTEM ASSUMPTIONS
/// ---------------------------------------------------------------------------
///
/// - Only ONE record per flash page
/// - Page size is sufficient for `N + 2 bytes CRC`
/// - Power-fail safety handled externally (hardware + system logic)
/// - Writes occur ONLY in safe context (never ISR)
///
const FLASH_BASE: u32 = 0x0800_0000;
const PAGE_ADDR: u32 = 0x0800_7800;

/// ---------------------------------------------------------------------------
/// 🧮 CRC16 (MODBUS STANDARD)
/// ---------------------------------------------------------------------------
///
/// Polynomial: 0xA001
/// Initial:    0xFFFF
///
#[inline(always)]
fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;

    for &b in data {
        crc ^= b as u16;

        for _ in 0..8 {
            crc = if (crc & 1) != 0 {
                (crc >> 1) ^ 0xA001
            } else {
                crc >> 1
            };
        }
    }

    crc
}

/// ---------------------------------------------------------------------------
/// 📦 FLASH RECORD FORMAT
/// ---------------------------------------------------------------------------
///
/// Stored layout in flash:
///
///     [ payload: [u8; N] ][ crc: u16 ]
///
#[repr(C)]
#[derive(Copy, Clone)]
pub struct FlashRecord<const N: usize> {
    pub data: [u8; N],
    pub crc: u16,
}

/// ---------------------------------------------------------------------------
/// 🧩 FLASH DRIVER (ZERO STATE)
/// ---------------------------------------------------------------------------
///
/// Stateless driver:
/// - No runtime state
/// - No initialization required
/// - Pure register-level operations
///
pub struct Flash;

impl Flash {
    /// Optional constructor (future extensibility)
    ///
    /// Currently does nothing, but allows:
    /// - multi-region flash systems
    /// - dependency injection
    ///
    #[inline(always)]
    pub const fn new() -> Self {
        Self
    }

    // -----------------------------------------------------------------------
    // 📖 READ FLASH
    // -----------------------------------------------------------------------
    ///
    /// Reads a `[u8; N]` blob from flash.
    ///
    /// ✔ CRC verified before return
    /// ✔ Returns None if data is invalid or uninitialized
    ///
    /// # Example
    ///
    /// ```rust
    /// let data: Option<[u8; 8]> = Flash::read();
    /// if let Some(buf) = data {
    ///     // valid flash content
    /// }
    /// ```
    ///
    #[inline(always)]
    pub fn read<const N: usize>() -> Option<[u8; N]> {
        let record = unsafe { &*(PAGE_ADDR as *const FlashRecord<N>) };

        let calc_crc = crc16(&record.data);

        if calc_crc == record.crc {
            Some(record.data)
        } else {
            None
        }
    }

    // -----------------------------------------------------------------------
    // ✍️ WRITE FLASH
    // -----------------------------------------------------------------------
    ///
    /// Writes a `[u8; N]` blob into flash memory.
    ///
    /// ⚠️ WARNING:
    /// - BLOCKING operation
    /// - Must NOT be called from ISR
    /// - Must only run in safe system context
    ///
    /// # Example
    ///
    /// ```rust
    /// use stm32c0::stm32c031 as pac;
    ///
    /// let flash = Flash::new();
    /// let dp = unsafe { pac::Peripherals::steal() };
    ///
    /// let data = [1u8, 2, 3, 4];
    /// Flash::write::<4>(&dp.FLASH, &data);
    /// ```
    ///
    #[inline(always)]
    pub fn write<const N: usize>(flash: &pac::FLASH, data: &[u8; N]) {
        let crc = crc16(data);

        let record = FlashRecord { data: *data, crc };

        unsafe {
            // -------------------------------------------------------
            // 🔓 UNLOCK FLASH
            // -------------------------------------------------------
            if flash.cr().read().lock().bit_is_set() {
                flash.keyr().write(|w| w.bits(0x4567_0123));
                flash.keyr().write(|w| w.bits(0xCDEF_89AB));
            }

            // -------------------------------------------------------
            // 🧹 ERASE PAGE (STM32C0 PAGE-BASED MODEL)
            // -------------------------------------------------------
            let page = (PAGE_ADDR - FLASH_BASE) / 2048; // 2KB page assumption

            flash.cr().modify(|_, w| w.per().set_bit());

            flash.cr().modify(|_, w| w.pnb().bits(page as u8));

            flash.cr().modify(|_, w| w.strt().set_bit());

            while flash.sr().read().bsy1().bit_is_set() {}

            // -------------------------------------------------------
            // ✍️ PROGRAM FLASH (HALF-WORD ACCESS)
            // -------------------------------------------------------
            let mut addr = PAGE_ADDR as *mut u16;
            let src = &record as *const _ as *const u16;

            let words = core::mem::size_of::<FlashRecord<N>>() / 2;

            for i in 0..words {
                flash.cr().modify(|_, w| w.pg().set_bit());

                ptr::write_volatile(addr, ptr::read_volatile(src.add(i)));

                while flash.sr().read().bsy1().bit_is_set() {}

                addr = addr.add(1);
            }

            flash.cr().modify(|_, w| w.pg().clear_bit());

            // -------------------------------------------------------
            // 🔒 LOCK FLASH
            // -------------------------------------------------------
            flash.cr().modify(|_, w| w.lock().set_bit());
        }
    }

    // -----------------------------------------------------------------------
    // 🧪 VALIDATION API
    // -----------------------------------------------------------------------
    ///
    /// Validates a flash record in memory.
    ///
    #[inline(always)]
    pub fn validate<const N: usize>(record: &FlashRecord<N>) -> bool {
        let data_bytes =
            unsafe { core::slice::from_raw_parts(&record.data as *const _ as *const u8, N) };

        crc16(data_bytes) == record.crc
    }
}
