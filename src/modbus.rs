use crate::drivers::uart::Uart;

const MAX_FRAME: usize = 256;

/// Modbus RTU core (ISR-safe, zero allocation)
pub struct Modbus {
    buf: [u8; MAX_FRAME],
    len: usize,
}

impl Modbus {
    pub const fn new() -> Self {
        Self {
            buf: [0; MAX_FRAME],
            len: 0,
        }
    }

    // ------------------------------------------------------------
    // RX byte (called from ISR)
    // ------------------------------------------------------------
    #[inline(always)]
    pub fn push_byte(&mut self, b: u8) {
        if self.len < MAX_FRAME {
            self.buf[self.len] = b;
            self.len += 1;
        } else {
            // overflow → invalidate frame
            self.len = 0;
        }
    }

    // ------------------------------------------------------------
    // Frame complete (called on RTO)
    // ------------------------------------------------------------
    pub fn process(&mut self, uart: &Uart) {
        let len = self.len;

        // Reset early → prevents race with RX
        self.len = 0;

        if len < 4 {
            return;
        }

        if !crc_ok(&self.buf[..len]) {
            return;
        }

        let addr = self.buf[0];
        let func = self.buf[1];

        const SLAVE_ID: u8 = 0x01;

        if addr != SLAVE_ID {
            return;
        }

        match func {
            0x03 => self.handle_read_holding(uart, len),
            0x06 => self.handle_write_single(uart),
            _ => self.exception(uart, func, 0x01),
        }
    }

    // ------------------------------------------------------------
    // 0x03 Read Holding Registers
    // ------------------------------------------------------------
    fn handle_read_holding(&self, uart: &Uart, len: usize) {
        if len < 8 {
            return;
        }

        let start = u16::from_be_bytes([self.buf[2], self.buf[3]]);
        let count = u16::from_be_bytes([self.buf[4], self.buf[5]]);

        if count == 0 || count > 32 {
            self.exception(uart, 0x03, 0x03);
            return;
        }

        let byte_count = (count * 2) as u8;

        let mut frame = [0u8; 3 + 64 + 2];
        let mut idx = 0;

        frame[idx] = self.buf[0]; // echo slave ID
        idx += 1;

        frame[idx] = 0x03;
        idx += 1;

        frame[idx] = byte_count;
        idx += 1;

        for i in 0..count {
            let val = read_register(start + i);
            frame[idx] = (val >> 8) as u8;
            idx += 1;
            frame[idx] = val as u8;
            idx += 1;
        }

        append_crc(&mut frame, &mut idx);

        send_frame(uart, &frame[..idx]);
    }

    // ------------------------------------------------------------
    // 0x06 Write Single Register
    // ------------------------------------------------------------
    fn handle_write_single(&self, uart: &Uart) {
        let reg = u16::from_be_bytes([self.buf[2], self.buf[3]]);
        let val = u16::from_be_bytes([self.buf[4], self.buf[5]]);

        write_register(reg, val);

        let mut frame = [0u8; 8];

        frame[..6].copy_from_slice(&self.buf[..6]);

        append_crc(&mut frame, &mut 6usize.clone());

        send_frame(uart, &frame);
    }

    // ------------------------------------------------------------
    // Exception response
    // ------------------------------------------------------------
    fn exception(&self, uart: &Uart, func: u8, code: u8) {
        let mut frame = [0u8; 5];

        frame[0] = self.buf[0];
        frame[1] = func | 0x80;
        frame[2] = code;

        let mut idx = 3;
        append_crc(&mut frame, &mut idx);

        send_frame(uart, &frame[..idx]);
    }
}

//
// ================= CRC =================
//

#[inline(always)]
fn crc_ok(frame: &[u8]) -> bool {
    let len = frame.len();
    let crc_calc = crc16(&frame[..len - 2]);
    let crc_rx = u16::from_le_bytes([frame[len - 2], frame[len - 1]]);
    crc_calc == crc_rx
}

#[inline(always)]
fn append_crc(buf: &mut [u8], idx: &mut usize) {
    let crc = crc16(&buf[..*idx]);

    buf[*idx] = (crc & 0xFF) as u8;
    *idx += 1;

    buf[*idx] = (crc >> 8) as u8;
    *idx += 1;
}

#[inline(always)]
fn send_frame(uart: &Uart, data: &[u8]) {
    for &b in data {
        uart.write_byte(b);
    }
    uart.flush();
}

#[inline(always)]
fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0xFFFF;

    for &b in data {
        crc ^= b as u16;

        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xA001
            } else {
                crc >> 1
            };
        }
    }

    crc
}

//
// ================= REGISTER MAP =================
//

fn read_register(addr: u16) -> u16 {
    match addr {
        0x0000 => 1234,
        0x0001 => 5678,
        _ => 0,
    }
}

fn write_register(addr: u16, val: u16) {
    match addr {
        0x0001 => {
            let _ = val;
        }
        _ => {}
    }
}