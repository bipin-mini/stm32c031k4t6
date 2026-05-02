use stm32c0::stm32c031 as pac;

const TX_BUF_SIZE: usize = 256;

/// UART driver for Modbus RTU over RS485
///
/// Target:
/// - MCU: STM32C031
/// - CPU Clock: **48 MHz (HSI)**
/// - Baud: **9600, 8N1**
///
/// Design goals:
/// - Deterministic ISR execution
/// - Zero allocation
/// - Non-blocking TX (interrupt driven)
/// - Hardware-assisted frame detection (RTO)
/// - RS485 DE handled by hardware (DEM)
///
/// ---------------------------------------------------------------------------
/// Timing Reference @ 48 MHz / 9600 baud:
///
/// Bit time      = 1 / 9600  ≈ 104.166 µs
/// Char time     = 10 bits   ≈ 1.041 ms (8N1)
/// Modbus gap    = 3.5 chars ≈ 3.645 ms
///
/// RTO is configured in *bit times* (NOT CPU cycles)
/// → independent of CPU frequency once baud is set.
///
/// ---------------------------------------------------------------------------
pub struct Uart {
    pub usart: pac::USART1,

    // ---------------- TX STATE ----------------
    /// Transmit buffer (ISR drained)
    tx_buf: [u8; TX_BUF_SIZE],

    /// Total bytes to send
    tx_len: usize,

    /// Current index (next byte to send)
    tx_idx: usize,

    /// TX active flag
    tx_busy: bool,
}

// ---------------------------------------------------------------------------
// ISR EVENT MODEL
// ---------------------------------------------------------------------------

/// Events emitted from UART ISR
///
/// Single-event model avoids borrow conflicts in RTIC
pub enum Event {
    /// Byte received
    Rx(u8),

    /// End of Modbus frame detected (via RTO)
    FrameEnd,

    /// Transmission fully completed (TC flag)
    TxDone,
}

impl Uart {
    /// Initialize USART1 for Modbus RTU
    ///
    /// # Arguments
    /// - `usart`: peripheral instance
    /// - `rcc`: clock control
    /// - `slave_id`: Modbus address (stored in ADD register, not used unless MME enabled)
    pub fn new(usart: pac::USART1, rcc: &pac::RCC, slave_id: u8) -> Self {
        // -------------------------------------------------------------------
        // Enable peripheral clock
        // -------------------------------------------------------------------
        rcc.apbenr2().modify(|_, w| w.usart1en().set_bit());

        // Disable USART before configuration
        usart.cr1().modify(|_, w| w.ue().clear_bit());

        // -------------------------------------------------------------------
        // Baud rate: 9600 @ 48 MHz
        //
        // BRR = Fclk / baud = 48_000_000 / 9600 = 5000 (exact)
        // → ZERO baud error
        // -------------------------------------------------------------------
        usart.brr().write(|w| unsafe { w.bits(48_000_000 / 9600) });

        // -------------------------------------------------------------------
        // Receiver Timeout (RTO)
        //
        // Used for Modbus RTU frame detection
        //
        // Value is in BIT TIMES:
        // 35 bits ≈ 3.5 characters (Modbus requirement)
        //
        // 35 * 104 µs ≈ 3.645 ms
        //
        // IMPORTANT:
        // - Independent of CPU frequency
        // - Based on baud clock
        // -------------------------------------------------------------------
        usart.rtor().write(|w| unsafe { w.rto().bits(35) });
        usart.cr2().modify(|_, w| w.rtoen().set_bit());

        // -------------------------------------------------------------------
        // RS485 Driver Enable (Hardware Controlled)
        //
        // DEM = 1 → automatic DE control
        // DEP = 0 → DE active HIGH
        //
        // DE timing:
        // - DEAT = 3 → assert 3 bit times before TX
        // - DEDT = 3 → release 3 bit times after TX
        //
        // @9600 baud:
        // 3 bits ≈ 312 µs
        //
        // Safe and robust for bus turn-around
        // -------------------------------------------------------------------
        usart
            .cr3()
            .modify(|_, w| w.dem().set_bit().dep().clear_bit());

        usart
            .cr1()
            .modify(|_, w| unsafe { w.deat().bits(3).dedt().bits(3) });

        // -------------------------------------------------------------------
        // Address register (future use)
        //
        // NOTE:
        // Multiprocessor mode (MME) is DISABLED,
        // so this does NOT filter frames in hardware.
        //
        // Filtering is done in software (Modbus layer)
        // -------------------------------------------------------------------
        usart.cr2().modify(|_, w| unsafe { w.add().bits(slave_id) });
        usart.cr1().modify(|_, w| w.mme().clear_bit());

        // -------------------------------------------------------------------
        // Enable TX, RX and interrupts
        //
        // RXNEIE → byte received
        // RTOIE  → frame timeout (Modbus frame end)
        // TCIE   → transmission complete (true end of frame)
        // -------------------------------------------------------------------
        usart.cr1().modify(|_, w| {
            w.re()
                .set_bit()
                .te()
                .set_bit()
                .rxneie()
                .set_bit()
                .rtoie()
                .set_bit()
                .tcie()
                .set_bit()
        });

        // Enable USART
        usart.cr1().modify(|_, w| w.ue().set_bit());

        Self {
            usart,
            tx_buf: [0; TX_BUF_SIZE],
            tx_len: 0,
            tx_idx: 0,
            tx_busy: false,
        }
    }

    // -----------------------------------------------------------------------
    // Start TX (NON-BLOCKING)
    // -----------------------------------------------------------------------

    /// Start transmission of a frame
    ///
    /// - Non-blocking
    /// - Data is copied into internal buffer
    /// - Transmission proceeds via ISR
    ///
    /// NOTE:
    /// - If TX is already active → frame is dropped
    pub fn start_tx(&mut self, data: &[u8]) {
        if self.tx_busy {
            // Design choice: drop if busy
            return;
        }

        let len = data.len().min(TX_BUF_SIZE);
        self.tx_buf[..len].copy_from_slice(&data[..len]);

        self.tx_len = len;
        self.tx_idx = 0;
        self.tx_busy = true;

        // Kickstart transmission if TX FIFO ready
        if self.usart.isr().read().txfnf().bit_is_set() {
            let b = self.tx_buf[self.tx_idx];
            self.tx_idx += 1;
            self.usart.tdr().write(|w| unsafe { w.bits(b as u32) });
        }

        // Enable TX interrupt for remaining bytes
        self.usart.cr1().modify(|_, w| w.txeie().set_bit());
    }

    // -----------------------------------------------------------------------
    // Runtime Modbus address update
    // -----------------------------------------------------------------------

    pub fn set_slave_id(&self, id: u8) {
        self.usart.cr2().modify(|_, w| unsafe { w.add().bits(id) });
    }

    // -----------------------------------------------------------------------
    // ISR Handler
    // -----------------------------------------------------------------------

    /// UART ISR handler
    ///
    /// Emits events:
    /// - Rx(byte)
    /// - FrameEnd (via RTO)
    /// - TxDone (via TC)
    ///
    /// Design:
    /// - No branching on application logic
    /// - Constant-time behavior
    /// - Minimal register access
    pub fn isr<F>(&mut self, mut f: F)
    where
        F: FnMut(Event),
    {
        let isr = self.usart.isr().read();

        // ---------------- RX ----------------
        if isr.rxfne().bit_is_set() {
            let b = self.usart.rdr().read().bits() as u8;
            f(Event::Rx(b));
        }

        // ---------------- FRAME END ----------------
        if isr.rtof().bit_is_set() {
            self.usart.icr().write(|w| w.rtocf().bit(true));
            f(Event::FrameEnd);
        }

        // ---------------- TX ----------------
        if isr.txfnf().bit_is_set() && self.tx_busy {
            if self.tx_idx < self.tx_len {
                let b = self.tx_buf[self.tx_idx];
                self.tx_idx += 1;
                self.usart.tdr().write(|w| unsafe { w.bits(b as u32) });
            } else {
                self.usart.cr1().modify(|_, w| w.txeie().clear_bit());
            }
        }

        // ---------------- TX COMPLETE ----------------
        if isr.tc().bit_is_set() {
            self.usart.icr().write(|w| w.tccf().bit(true));
            self.tx_busy = false;

            f(Event::TxDone);
        }

        // ---------------- ERROR HANDLING ----------------
        if isr.ore().bit_is_set() {
            self.usart.icr().write(|w| w.orecf().bit(true));
        }
        if isr.fe().bit_is_set() {
            self.usart.icr().write(|w| w.fecf().bit(true));
        }
        if isr.ne().bit_is_set() {
            self.usart.icr().write(|w| w.necf().bit(true));
        }
    }
}
