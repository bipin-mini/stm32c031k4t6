use crate::flash::{FlashError, Stm32Flash};

/// ---------------------------------------------------------------------------
/// EEPROM CONFIGURATION (STM32C031 SPECIFIC)
/// ---------------------------------------------------------------------------
///
/// Two-page ping-pong EEPROM emulation.
/// Each page is used as an append-only log.
///
/// IMPORTANT DESIGN RULES:
/// - Last slot of page is RESERVED for commit marker
/// - Data is NEVER written into last slot
/// - Page is considered VALID only if commit marker exists
///
const PAGE_A: u32 = 0x0803_8000;
const PAGE_B: u32 = 0x0803_A000;
const PAGE_SIZE: u32 = 2048;

/// Each entry = 64-bit
const SLOT_SIZE: u32 = 8;

/// Erased flash value
const ERASED: u64 = u64::MAX;

/// Page commit marker (must be unique & never valid data)
const PAGE_MAGIC: u64 = 0xA5A5_F0F0_5A5A_F0F0;

/// Last usable slot index
const LAST_SLOT_OFFSET: u32 = PAGE_SIZE - SLOT_SIZE;

/// ---------------------------------------------------------------------------
/// EEPROM STRUCTURE
/// ---------------------------------------------------------------------------
///
/// Maintains:
/// - Active page
/// - Write pointer (next free slot)
///
/// Guarantees:
/// - Power-fail safe append
/// - Page swap only during controlled operations
///
pub struct Eeprom {
    flash: Stm32Flash,
    active_page: u32,
    write_ptr: u32,
}

impl Eeprom {

    // ================================================================
    // INIT (BOOT RECOVERY)
    // ================================================================
    ///
    /// Recovery logic:
    /// 1. Scan both pages
    /// 2. Prefer committed page
    /// 3. Fallback to most filled page
    ///
    pub fn new(flash: Stm32Flash) -> Self {
        let (a_ptr, a_valid) = Self::scan_page(PAGE_A);
        let (b_ptr, b_valid) = Self::scan_page(PAGE_B);

        let (active_page, write_ptr) = match (a_valid, b_valid) {
            (true, false) => (PAGE_A, a_ptr),
            (false, true) => (PAGE_B, b_ptr),

            // both valid OR both invalid → choose most recent (longer log)
            _ => {
                if a_ptr >= b_ptr {
                    (PAGE_A, a_ptr)
                } else {
                    (PAGE_B, b_ptr)
                }
            }
        };

        Self {
            flash,
            active_page,
            write_ptr,
        }
    }

    // ================================================================
    // PAGE SCAN
    // ================================================================
    ///
    /// Returns:
    /// - next write pointer
    /// - whether page is COMMITTED
    ///
    fn scan_page(page: u32) -> (u32, bool) {
        let mut addr = page;
        let end = page + LAST_SLOT_OFFSET; // exclude commit slot

        while addr < end {
            let word = unsafe { core::ptr::read_volatile(addr as *const u64) };

            if word == ERASED {
                break;
            }

            if Self::decode(word).is_none() {
                break;
            }

            addr += SLOT_SIZE;
        }

        let committed = Self::is_page_committed(page);

        (addr, committed)
    }

    // ================================================================
    // COMMIT CHECK
    // ================================================================
    ///
    /// A page is VALID only if commit marker exists at last slot
    ///
    fn is_page_committed(page: u32) -> bool {
        let addr = page + LAST_SLOT_OFFSET;
        let word = unsafe { core::ptr::read_volatile(addr as *const u64) };
        word == PAGE_MAGIC
    }

    // ================================================================
    // ENCODE / DECODE
    // ================================================================
    ///
    /// Simple format:
    /// [ ID (8-bit) | VALUE (56-bit) ]
    ///
    #[inline(always)]
    fn encode(id: u8, value: u64) -> u64 {
        ((id as u64) << 56) | (value & 0x00FF_FFFF_FFFF_FFFF)
    }

    #[inline(always)]
    fn decode(word: u64) -> Option<(u8, u64)> {
        if word == ERASED || word == PAGE_MAGIC {
            return None;
        }

        let id = (word >> 56) as u8;
        let value = word & 0x00FF_FFFF_FFFF_FFFF;

        Some((id, value))
    }

    // ================================================================
    // READ LAST VALUE
    // ================================================================
    ///
    /// Reverse scan → returns most recent value
    ///
    pub fn read(&self, id: u8) -> Option<u64> {
        let mut addr = self.write_ptr;

        while addr > self.active_page {
            addr -= SLOT_SIZE;

            let word = unsafe { core::ptr::read_volatile(addr as *const u64) };

            if let Some((i, v)) = Self::decode(word) {
                if i == id {
                    return Some(v);
                }
            }
        }

        None
    }

    // ================================================================
    // WRITE ENTRY
    // ================================================================
    ///
    /// Automatically triggers page swap if needed
    ///
    pub fn write(&mut self, id: u8, value: u64) -> Result<(), FlashError> {
        if self.is_near_full() {
            return self.swap_pages(id, value);
        }

        self.append(id, value)
    }

    // ================================================================
    // APPEND ENTRY
    // ================================================================
    ///
    /// Writes single slot safely
    ///
    fn append(&mut self, id: u8, value: u64) -> Result<(), FlashError> {
        let addr = self.write_ptr;

        self.flash.write_double_word(addr, Self::encode(id, value))?;

        self.write_ptr += SLOT_SIZE;

        Ok(())
    }

    // ================================================================
    // CAPACITY CHECK
    // ================================================================
    ///
    /// Ensures:
    /// - space for at least ONE data slot
    /// - space for commit marker
    ///
    fn is_near_full(&self) -> bool {
        self.write_ptr >= self.active_page + LAST_SLOT_OFFSET - SLOT_SIZE
    }

    // ================================================================
    // PAGE SWAP (SAFE + ATOMIC)
    // ================================================================
    ///
    /// Steps:
    /// 1. Erase new page
    /// 2. Copy valid entries
    /// 3. Write new value
    /// 4. WRITE COMMIT MARKER (FINAL STEP)
    ///
    /// Power-fail safety:
    /// - If commit not written → page ignored
    /// - Old page remains valid
    ///
    fn swap_pages(&mut self, id: u8, value: u64) -> Result<(), FlashError> {
        let new_page = if self.active_page == PAGE_A {
            PAGE_B
        } else {
            PAGE_A
        };

        // 1. ERASE NEW PAGE
        self.flash.erase_page(new_page)?;

        let mut dst = new_page;
        let mut src = self.active_page;

        // 2. COPY VALID ENTRIES
        while src < self.write_ptr {
            let word = unsafe { core::ptr::read_volatile(src as *const u64) };

            if let Some((i, v)) = Self::decode(word) {
                if i != id {
                    self.flash.write_double_word(dst, Self::encode(i, v))?;
                    dst += SLOT_SIZE;
                }
            }

            src += SLOT_SIZE;
        }

        // 3. WRITE NEW VALUE
        self.flash.write_double_word(dst, Self::encode(id, value))?;
        dst += SLOT_SIZE;

        // 4. WRITE COMMIT MARKER (CRITICAL ATOMIC POINT)
        let commit_addr = new_page + LAST_SLOT_OFFSET;
        self.flash.write_double_word(commit_addr, PAGE_MAGIC)?;

        // 5. SWITCH ACTIVE PAGE
        self.active_page = new_page;
        self.write_ptr = dst;

        Ok(())
    }
}