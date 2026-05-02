use crate::drivers::uart::Uart;

const MAX_FRAME: usize = 256;
const DEFAULT_SLAVE_ID: u8 = 247;

/// ---------------------------------------------------------------------------
/// Modbus RTU core (ISR-safe, zero allocation, 48 MHz optimized)
///
/// Design goals:
/// - Zero dynamic allocation
/// - ISR does minimal work (only buffering)
/// - Main loop does full protocol processing
/// - Deterministic execution
/// - Safe without impacting high-priority interrupts (encoder)
///
/// Timing context (48 MHz CPU):
/// - 1 cycle ≈ 20.8 ns
/// - Critical section used here: ~20–40 cycles (< 1 µs)
/// - Modbus @9600 baud: 1 byte ≈ 1 ms
///
/// → Conclusion:
/// Critical section is negligible and safe even with fast encoder ISRs
/// ---------------------------------------------------------------------------
pub struct Modbus {
    buf: [u8; MAX_FRAME],
    len: usize,
    frame_ready: bool,
    slave_id: u8,
}

impl Modbus {
    /// -----------------------------------------------------------------------
    /// Initialize Modbus stack
    ///
    /// - Reads slave ID from EEPROM (stub for now)
    /// - Validates range (1..=247)
    /// - Defaults to 247 if invalid
    /// -----------------------------------------------------------------------
    pub fn new() -> Self {
        let id = validate_slave_id(read_slave_id_from_eeprom());

        Self {
            buf: [0; MAX_FRAME],
            len: 0,
            frame_ready: false,
            slave_id: id,
        }
    }

    /// Get current slave ID
    #[inline(always)]
    pub fn slave_id(&self) -> u8 {
        self.slave_id
    }

    /// -----------------------------------------------------------------------
    /// Update slave ID at runtime
    ///
    /// - Keeps UART hardware in sync
    /// - Future: call EEPROM write here
    /// -----------------------------------------------------------------------
    pub fn set_slave_id(&mut self, id: u8, uart: &mut Uart) {
        let id = validate_slave_id(id);
        self.slave_id = id;

        // Update UART (even if MME disabled, keeps future-proof)
        uart.set_slave_id(id);
    }

    // -----------------------------------------------------------------------
    // ISR: RX byte handler
    //
    // Rules:
    // - NO branching explosion
    // - NO heavy logic
    // - NO CRC
    // -----------------------------------------------------------------------
    #[inline(always)]
    pub fn push_byte(&mut self, b: u8) {
        // If frame not yet processed → ignore new data
        if self.frame_ready {
            return;
        }

        if self.len < MAX_FRAME {
            self.buf[self.len] = b;
            self.len += 1;
        } else {
            // Overflow → invalidate frame cleanly
            self.len = 0;
        }
    }

    // -----------------------------------------------------------------------
    // ISR: Frame complete (USART RTO interrupt)
    // -----------------------------------------------------------------------
    #[inline(always)]
    pub fn frame_complete(&mut self) {
        if self.len > 0 {
            self.frame_ready = true;
        }
    }

    // -----------------------------------------------------------------------
    // MAIN LOOP: Process frame (non-ISR context)
    //
    // IMPORTANT:
    // - Uses very short critical section (~<1 µs @48MHz)
    // - Prevents race with ISR
    // - No full buffer copy → zero overhead
    // -----------------------------------------------------------------------
    pub fn poll(&mut self, uart: &mut Uart) {
        let mut len = 0;

        cortex_m::interrupt::free(|_| {
            if self.frame_ready {
                len = self.len;

                // Reset early → ISR can start next frame
                self.frame_ready = false;
                self.len = 0;
            }
        });

        // Nothing to process
        if len < 8 {
            return;
        }

        let frame = &self.buf[..len];

        // ---------------- Address filter (fast reject) ----------------
        let addr = frame[0];

        // Broadcast (0) allowed
        if addr != self.slave_id && addr != 0 {
            return;
        }

        // ---------------- CRC check ----------------
        if !crc_ok(frame) {
            return;
        }

        let func = frame[1];

        match func {
            0x03 => self.handle_read(uart, frame),
            0x06 => self.handle_write(uart, frame, addr),
            _ => self.exception(uart, func, 0x01),
        }
    }

    // -----------------------------------------------------------------------
    // 0x03 Read Holding Registers
    // -----------------------------------------------------------------------
    fn handle_read(&self, uart: &mut Uart, frame: &[u8]) {
        if frame.len() != 8 {
            return;
        }

        let start = u16::from_be_bytes([frame[2], frame[3]]);
        let count = u16::from_be_bytes([frame[4], frame[5]]);

        if count == 0 || count > 32 {
            self.exception(uart, 0x03, 0x03);
            return;
        }

        // Validate all registers FIRST (deterministic)
        for i in 0..count {
            if !valid_register(start + i) {
                self.exception(uart, 0x03, 0x02);
                return;
            }
        }

        let mut resp = [0u8; 3 + 64 + 2];
        let mut idx = 0;

        resp[idx] = self.slave_id;
        idx += 1;

        resp[idx] = 0x03;
        idx += 1;

        resp[idx] = (count * 2) as u8;
        idx += 1;

        for i in 0..count {
            let val = read_register(start + i);

            resp[idx] = (val >> 8) as u8;
            idx += 1;

            resp[idx] = val as u8;
            idx += 1;
        }

        append_crc(&mut resp, &mut idx);
        uart.start_tx(&resp[..idx]);
    }

    // -----------------------------------------------------------------------
    // 0x06 Write Single Register
    // -----------------------------------------------------------------------
    fn handle_write(&self, uart: &mut Uart, frame: &[u8], addr: u8) {
        let reg = u16::from_be_bytes([frame[2], frame[3]]);
        let val = u16::from_be_bytes([frame[4], frame[5]]);

        if !valid_register(reg) {
            self.exception(uart, 0x06, 0x02);
            return;
        }

        write_register(reg, val);

        // Broadcast → NO RESPONSE
        if addr == 0 {
            return;
        }

        let mut resp = [0u8; 8];
        resp[..6].copy_from_slice(&frame[..6]);

        let mut idx = 6;
        append_crc(&mut resp, &mut idx);

        uart.start_tx(&resp);
    }

    // -----------------------------------------------------------------------
    // Exception response
    // -----------------------------------------------------------------------
    fn exception(&self, uart: &mut Uart, func: u8, code: u8) {
        let mut frame = [0u8; 5];

        frame[0] = self.slave_id;
        frame[1] = func | 0x80;
        frame[2] = code;

        let mut idx = 3;
        append_crc(&mut frame, &mut idx);

        uart.start_tx(&frame[..idx]);
    }
}

//
// ================= EEPROM STUB =================
//

fn read_slave_id_from_eeprom() -> u8 {
    DEFAULT_SLAVE_ID
}

fn validate_slave_id(id: u8) -> u8 {
    if (1..=247).contains(&id) {
        id
    } else {
        DEFAULT_SLAVE_ID
    }
}

//
// ================= CRC =================
//

fn crc_ok(frame: &[u8]) -> bool {
    let len = frame.len();
    let crc_calc = crc16(&frame[..len - 2]);
    let crc_rx = u16::from_le_bytes([frame[len - 2], frame[len - 1]]);
    crc_calc == crc_rx
}

fn append_crc(buf: &mut [u8], idx: &mut usize) {
    let crc = crc16(&buf[..*idx]);

    buf[*idx] = (crc & 0xFF) as u8;
    *idx += 1;

    buf[*idx] = (crc >> 8) as u8;
    *idx += 1;
}

fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0xFFFF;

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

//
// ================= REGISTER MAP (stub) =================
//

fn valid_register(addr: u16) -> bool {
    matches!(addr, 0x0000 | 0x0001)
}

fn read_register(addr: u16) -> u16 {
    match addr {
        0x0000 => 1234,
        0x0001 => 5678,
        _ => 0,
    }
}

fn write_register(_addr: u16, _val: u16) {}
