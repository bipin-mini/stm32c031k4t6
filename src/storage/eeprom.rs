use crate::bsp::eeprom as cfg;
use crate::flash::{FlashError, Stm32Flash};

const ERASED: u16 = 0xFFFF;
const RECEIVE_DATA: u16 = 0xEEEE;
const VALID_PAGE: u16 = 0x0000;

const ERASED_DW: u64 = 0xFFFF_FFFF_FFFF_FFFF;

pub struct Eeprom {
    flash: Stm32Flash,
    active_page: u32,

    /// Next free write location (O(1) writes)
    write_ptr: u32,
}

impl Eeprom {
    pub fn new(mut flash: Stm32Flash) -> Result<Self, FlashError> {
        let p0 = Self::read_status(&flash, cfg::PAGE0_BASE);
        let p1 = Self::read_status(&flash, cfg::PAGE1_BASE);

        let active = match (p0, p1) {
            (VALID_PAGE, ERASED) => cfg::PAGE0_BASE,
            (ERASED, VALID_PAGE) => cfg::PAGE1_BASE,

            (RECEIVE_DATA, VALID_PAGE) => {
                Self::recover(&mut flash, cfg::PAGE0_BASE, cfg::PAGE1_BASE)?;
                cfg::PAGE0_BASE
            }
            (VALID_PAGE, RECEIVE_DATA) => {
                Self::recover(&mut flash, cfg::PAGE1_BASE, cfg::PAGE0_BASE)?;
                cfg::PAGE1_BASE
            }

            _ => {
                flash.erase_page(cfg::PAGE0_BASE)?;
                flash.write_double_word(cfg::PAGE0_BASE, Self::header_word(VALID_PAGE))?;
                cfg::PAGE0_BASE
            }
        };

        let write_ptr = Self::find_write_ptr(&flash, active);

        Ok(Self {
            flash,
            active_page: active,
            write_ptr,
        })
    }

    #[inline(always)]
    fn header_word(status: u16) -> u64 {
        (ERASED_DW & !0xFFFF) | (status as u64)
    }

    #[inline(always)]
    fn read_status(flash: &Stm32Flash, base: u32) -> u16 {
        flash.read_word(base) as u16
    }

    #[inline(always)]
    fn read_dw(&self, addr: u32) -> u64 {
        let low = self.flash.read_word(addr) as u64;
        let high = self.flash.read_word(addr + 4) as u64;
        (high << 32) | low
    }

    /// ---------------------------------------------------------------
    /// FAST WRITE POINTER INIT (boot-time scan)
    /// ---------------------------------------------------------------
    fn find_write_ptr(flash: &Stm32Flash, page: u32) -> u32 {
        let mut addr = page + 8;

        while addr < page + cfg::PAGE_SIZE as u32 {
            let low = flash.read_word(addr);
            let high = flash.read_word(addr + 4);

            if ((high as u64) << 32 | low as u64) == ERASED_DW {
                return addr;
            }

            addr += 8;
        }

        addr // page full
    }

    fn recover(flash: &mut Stm32Flash, new_page: u32, old_page: u32) -> Result<(), FlashError> {
        flash.write_double_word(new_page, Self::header_word(VALID_PAGE))?;
        flash.erase_page(old_page)?;
        Ok(())
    }

    pub fn read(&self, virt_addr: u16) -> Option<u8> {
        let mut addr = self.write_ptr;

        while addr > self.active_page {
            addr -= 8;

            let dw = self.read_dw(addr);

            if dw == ERASED_DW {
                continue;
            }

            let va = (dw & 0xFFFF) as u16;
            let val = ((dw >> 16) & 0xFF) as u8;

            if va == virt_addr {
                return Some(val);
            }
        }

        None
    }

    pub fn write(&mut self, virt_addr: u16, value: u8) -> Result<(), FlashError> {
        if self.read(virt_addr) == Some(value) {
            return Ok(());
        }

        // 🔴 TRANSFER GUARD (critical)
        if !self.has_space() {
            return self.page_transfer(virt_addr, value);
        }

        self.append_fast(virt_addr, value)
    }

    #[inline(always)]
    fn has_space(&self) -> bool {
        self.write_ptr < self.active_page + cfg::PAGE_SIZE as u32
    }

    #[inline(always)]
    fn encode(virt_addr: u16, value: u8) -> u64 {
        (virt_addr as u64) | ((value as u64) << 16)
    }

    /// ---------------------------------------------------------------
    /// O(1) append
    /// ---------------------------------------------------------------
    fn append_fast(&mut self, virt_addr: u16, value: u8) -> Result<(), FlashError> {
        let addr = self.write_ptr;

        if addr >= self.active_page + cfg::PAGE_SIZE as u32 {
            return Err(FlashError::ProgramError);
        }

        self.flash
            .write_double_word(addr, Self::encode(virt_addr, value))?;

        self.write_ptr += 8;

        Ok(())
    }

    fn append(&mut self, page: u32, virt_addr: u16, value: u8) -> Result<(), FlashError> {
        let mut addr = page + 8;

        while addr < page + cfg::PAGE_SIZE as u32 {
            if self.read_dw(addr) == ERASED_DW {
                return self.flash.write_double_word(addr, Self::encode(virt_addr, value));
            }
            addr += 8;
        }

        Ok(())
    }

    fn page_transfer(&mut self, virt_addr: u16, value: u8) -> Result<(), FlashError> {
        let new_page = if self.active_page == cfg::PAGE0_BASE {
            cfg::PAGE1_BASE
        } else {
            cfg::PAGE0_BASE
        };

        self.flash.erase_page(new_page)?;

        self.flash
            .write_double_word(new_page, Self::header_word(RECEIVE_DATA))?;

        for i in 0..cfg::EEPROM_SIZE {
            let va = i as u16;

            if va == virt_addr {
                continue;
            }

            if let Some(v) = self.read(va) {
                self.append(new_page, va, v)?;
            }
        }

        self.append(new_page, virt_addr, value)?;

        self.flash
            .write_double_word(new_page, Self::header_word(VALID_PAGE))?;

        self.flash.erase_page(self.active_page)?;

        self.active_page = new_page;

        // 🔴 recompute write pointer
        self.write_ptr = Self::find_write_ptr(&self.flash, new_page);

        Ok(())
    }

    /// ---------------------------------------------------------------
    /// POWER FAIL SAFE WRITE (STRICT)
    /// ---------------------------------------------------------------
    ///
    /// - O(1)
    /// - No scan
    /// - No erase
    ///
    pub fn write_power_fail(&mut self, virt_addr: u16, value: u8) -> Result<(), FlashError> {
        let addr = self.write_ptr;

        if addr >= self.active_page + cfg::PAGE_SIZE as u32 {
            return Err(FlashError::ProgramError);
        }

        self.flash
            .write_double_word(addr, Self::encode(virt_addr, value))?;

        // no pointer increment needed (system halts after)

        Ok(())
    }
}