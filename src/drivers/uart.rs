use stm32c0::stm32c031 as pac;

pub struct Uart {
    pub usart: pac::USART1,
}

impl Uart {
    pub fn new(usart: pac::USART1, rcc: &pac::RCC) -> Self {
        // -------------------------------
        // Enable clock
        // -------------------------------
        rcc.apbenr2().modify(|_, w| w.usart1en().set_bit());

        // Disable USART before config
        usart.cr1().modify(|_, w| w.ue().clear_bit());

        // -------------------------------
        // Baud rate: 9600 @ 48 MHz
        // -------------------------------
        usart.brr().write(|w| unsafe { w.bits(48_000_000 / 9600) });

        // -------------------------------
        // Receiver timeout (Modbus RTU)
        // 3.5 chars ≈ 35 bits (8N1)
        // -------------------------------
        usart.rtor().write(|w| unsafe { w.rto().bits(35) });
        usart.cr2().modify(|_, w| w.rtoen().set_bit());

        // -------------------------------
        // RS485 Hardware Driver Enable
        // -------------------------------
        usart.cr3().modify(|_, w| {
            w.dem()
                .set_bit() // auto DE
                .dep()
                .clear_bit() // active HIGH
        });

        // DE timing (bit times)
        usart
            .cr1()
            .modify(|_, w| unsafe { w.deat().bits(1).dedt().bits(1) });

        // -------------------------------
        // Enable TX, RX, interrupts
        // -------------------------------
        usart.cr1().modify(|_, w| {
            w.te()
                .set_bit()
                .re()
                .set_bit()
                .rxneie()
                .set_bit()
                .rtoie()
                .set_bit()
                .tcie()
                .set_bit()
        });

        // -------------------------------
        // Enable USART
        // -------------------------------
        usart.cr1().modify(|_, w| w.ue().set_bit());

        Self { usart }
    }

    // -----------------------------------------------------------------------
    // TX (blocking, deterministic)
    // -----------------------------------------------------------------------
    #[inline(always)]
    pub fn write_byte(&self, b: u8) {
        while self.usart.isr().read().txfnf().bit_is_clear() {}
        self.usart.tdr().write(|w| unsafe { w.bits(b as u32) });
    }

    #[inline(always)]
    pub fn flush(&self) {
        while self.usart.isr().read().tc().bit_is_clear() {}
    }

    // -----------------------------------------------------------------------
    // ISR handler (core of driver)
    // -----------------------------------------------------------------------
    #[inline(always)]
    pub fn isr<F1, F2>(&mut self, mut on_rx: F1, mut on_frame: F2)
    where
        F1: FnMut(u8),
        F2: FnMut(),
    {
        let isr = self.usart.isr().read();

        // -------------------------------
        // RXNE → byte received
        // -------------------------------
        if isr.rxfne().bit_is_set() {
            let b = self.usart.rdr().read().bits() as u8;
            on_rx(b);
        }

        // -------------------------------
        // RTO → frame complete
        // -------------------------------
        if isr.rtof().bit_is_set() {
            self.usart.icr().write(|w| w.rtocf().clear());

            on_frame();
        }

        // -------------------------------
        // TC → transmission complete
        // (DE handled automatically)
        // -------------------------------
        if isr.tc().bit_is_set() {
            self.usart.icr().write(|w| w.tccf().clear());
        }

        // -------------------------------
        // Error handling (CRITICAL)
        // -------------------------------
        if isr.ore().bit_is_set() {
            self.usart.icr().write(|w| w.orecf().clear());
        }

        if isr.fe().bit_is_set() {
            self.usart.icr().write(|w| w.fecf().clear());
        }

        if isr.ne().bit_is_set() {
            self.usart.icr().write(|w| w.necf().clear());
        }
    }
}
