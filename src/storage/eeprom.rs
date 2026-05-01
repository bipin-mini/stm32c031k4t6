use stm32c0::stm32c031 as pac;

/// ---------------------------------------------------------------------------
/// 24C02 EEPROM Driver (I2C1, PAC only)
/// ---------------------------------------------------------------------------
///
/// Device: ST24C02 (256 bytes)
/// Page size: 8 bytes
/// Write cycle: ~5 ms (handled via ACK polling)
///
/// ---------------------------------------------------------------------------
const EEPROM_ADDR: u8 = 0x50;
const PAGE_SIZE: usize = 8;

pub struct Eeprom {
    i2c: pac::I2C1,
}

impl Eeprom {
    /// -----------------------------------------------------------------------
    /// Init I2C1 (Standard Mode 100kHz @ 48 MHz)
    /// -----------------------------------------------------------------------
    pub fn new(i2c: pac::I2C1, rcc: &pac::RCC) -> Self {
        // Enable clock
        rcc.apbenr1().modify(|_, w| w.i2c1en().set_bit());

        // Reset peripheral
        rcc.apbrstr1().modify(|_, w| w.i2c1rst().set_bit());
        rcc.apbrstr1().modify(|_, w| w.i2c1rst().clear_bit());

        // Timing (100kHz @ 48MHz)
        i2c.timingr().write(|w| unsafe { w.bits(0x20303E5D) });

        // Enable I2C
        i2c.cr1().modify(|_, w| w.pe().set_bit());

        Self { i2c }
    }

    /// -----------------------------------------------------------------------
    /// Write buffer (handles page boundaries)
    /// -----------------------------------------------------------------------
    pub fn write(&self, mut addr: u8, data: &[u8]) {
        cortex_m::interrupt::free(|_| {
            let mut offset = 0;

            while offset < data.len() {
                let page_offset = (addr as usize) % PAGE_SIZE;
                let space = PAGE_SIZE - page_offset;
                let chunk = core::cmp::min(space, data.len() - offset);

                self.write_page(addr, &data[offset..offset + chunk]);

                addr += chunk as u8;
                offset += chunk;

                self.wait_write_cycle();
            }
        });
    }

    /// -----------------------------------------------------------------------
    /// Write single page (max 8 bytes)
    /// -----------------------------------------------------------------------
    fn write_page(&self, mem_addr: u8, data: &[u8]) {
        let i2c = &self.i2c;

        // Wait bus free
        while i2c.isr().read().busy().bit_is_set() {}

        // START (write, no AUTOEND)
        i2c.cr2().write(|w| unsafe {
            w.sadd()
                .bits((EEPROM_ADDR << 1) as u16)
                .nbytes()
                .bits((data.len() + 1) as u8)
                .rd_wrn()
                .clear_bit()
                .autoend()
                .clear_bit()
                .start()
                .set_bit()
        });

        // Send memory address
        while i2c.isr().read().txis().bit_is_clear() {}
        i2c.txdr().write(|w| unsafe { w.bits(mem_addr as u32) });

        // Send data
        for &b in data {
            while i2c.isr().read().txis().bit_is_clear() {}
            i2c.txdr().write(|w| unsafe { w.bits(b as u32) });
        }

        // Wait transfer complete
        while i2c.isr().read().tc().bit_is_clear() {}

        // Generate STOP
        i2c.cr2().modify(|_, w| w.stop().set_bit());

        while i2c.isr().read().stopf().bit_is_clear() {}
        i2c.icr().write(|w| w.stopcf().clear());
    }

    /// -----------------------------------------------------------------------
    /// Read buffer
    /// -----------------------------------------------------------------------
    pub fn read(&self, addr: u8, buf: &mut [u8]) {
        let i2c = &self.i2c;

        while i2c.isr().read().busy().bit_is_set() {}

        // Write memory address
        i2c.cr2().write(|w| unsafe {
            w.sadd()
                .bits((EEPROM_ADDR << 1) as u16)
                .nbytes()
                .bits(1)
                .rd_wrn()
                .clear_bit()
                .autoend()
                .clear_bit()
                .start()
                .set_bit()
        });

        while i2c.isr().read().txis().bit_is_clear() {}
        i2c.txdr().write(|w| unsafe { w.bits(addr as u32) });

        while i2c.isr().read().tc().bit_is_clear() {}

        // Read phase
        i2c.cr2().write(|w| unsafe {
            w.sadd()
                .bits((EEPROM_ADDR << 1) as u16)
                .nbytes()
                .bits(buf.len() as u8)
                .rd_wrn()
                .set_bit()
                .autoend()
                .set_bit()
                .start()
                .set_bit()
        });

        for b in buf.iter_mut() {
            while i2c.isr().read().rxne().bit_is_clear() {}
            *b = i2c.rxdr().read().bits() as u8;
        }

        while i2c.isr().read().stopf().bit_is_clear() {}
        i2c.icr().write(|w| w.stopcf().clear());
    }

    /// -----------------------------------------------------------------------
    /// ACK polling (wait for internal write completion)
    /// -----------------------------------------------------------------------
    fn wait_write_cycle(&self) {
        let i2c = &self.i2c;

        loop {
            while i2c.isr().read().busy().bit_is_set() {}

            // Try addressing device
            i2c.cr2().write(|w| unsafe {
                w.sadd()
                    .bits((EEPROM_ADDR << 1) as u16)
                    .nbytes()
                    .bits(0)
                    .rd_wrn()
                    .clear_bit()
                    .start()
                    .set_bit()
            });

            let isr = i2c.isr().read();

            if isr.nackf().bit_is_clear() {
                // Device responded → STOP
                i2c.cr2().modify(|_, w| w.stop().set_bit());

                while i2c.isr().read().stopf().bit_is_clear() {}
                i2c.icr().write(|w| w.stopcf().clear());
                break;
            }

            // Clear NACK and retry
            i2c.icr().write(|w| w.nackcf().clear());
        }
    }
}
