#![allow(dead_code)]

use core::cell::UnsafeCell;
use stm32c0::stm32c031 as pac;

use crate::usart1;

/// ---------------------------------------------------------------------------
/// Minimal Modbus RTU (Spec-Compliant Frame Handling)
/// ---------------------------------------------------------------------------
///
/// ✔ Frame detection via 3.5 char silence (external timer REQUIRED)
/// ✔ Deterministic processing
/// ✔ No parsing until full frame received
///
/// ---------------------------------------------------------------------------

const SLAVE_ADDR: u8 = 1;
const REG_COUNT: usize = 16;

/// ---------------------------------------------------------------------------
/// Holding Registers
/// ---------------------------------------------------------------------------
struct HoldingRegs(UnsafeCell<[u16; REG_COUNT]>);
unsafe impl Sync for HoldingRegs {}

static HOLDING_REGS: HoldingRegs = HoldingRegs(UnsafeCell::new([0; REG_COUNT]));

/// ---------------------------------------------------------------------------
/// Frame Buffer
/// ---------------------------------------------------------------------------
struct FrameBuf {
    buf: [u8; 256],
    len: usize,
    ready: bool,
}

impl FrameBuf {
    const fn new() -> Self {
        Self {
            buf: [0; 256],
            len: 0,
            ready: false,
        }
    }

    #[inline(always)]
    fn reset(&mut self) {
        self.len = 0;
        self.ready = false;
    }

    #[inline(always)]
    fn push(&mut self, b: u8) {
        if self.len < self.buf.len() {
            self.buf[self.len] = b;
            self.len += 1;
        } else {
            self.reset();
        }
    }
}

struct MbState(UnsafeCell<FrameBuf>);
unsafe impl Sync for MbState {}

static MB: MbState = MbState(UnsafeCell::new(FrameBuf::new()));

/// ---------------------------------------------------------------------------
/// CRC16
/// ---------------------------------------------------------------------------
fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0xFFFF;

    for &b in data {
        crc ^= b as u16;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xA001;
            } else {
                crc >>= 1;
            }
        }
    }

    crc
}

/// ---------------------------------------------------------------------------
/// RX Byte Pump (call every cycle)
/// ---------------------------------------------------------------------------
///
/// ONLY moves bytes into frame buffer.
/// Does NOT process frames.
///
pub fn rx_pump() {
    let mb = unsafe { &mut *MB.0.get() };

    while let Some(b) = usart1::read() {
        mb.push(b);

        // IMPORTANT:
        // Timer must be reset externally here
        // (handled in main or timer ISR)
    }
}

/// ---------------------------------------------------------------------------
/// Frame Ready Trigger (called from timer ISR/task)
/// ---------------------------------------------------------------------------
///
/// Called when 3.5 char silence detected.
///
pub fn frame_ready() {
    let mb = unsafe { &mut *MB.0.get() };

    if mb.len >= 4 {
        mb.ready = true;
    } else {
        mb.reset();
    }
}

/// ---------------------------------------------------------------------------
/// Process Frame (call from RTIC task)
/// ---------------------------------------------------------------------------
pub fn process(usart1: &pac::USART1) {
    let mb = unsafe { &mut *MB.0.get() };

    if !mb.ready {
        return;
    }

    let frame = &mb.buf[..mb.len];

    // CRC check
    let crc_rx = u16::from_le_bytes([frame[mb.len - 2], frame[mb.len - 1]]);
    let crc_calc = crc16(&frame[..mb.len - 2]);

    if crc_rx != crc_calc {
        mb.reset();
        return;
    }

    // Address filter
    if frame[0] != SLAVE_ADDR {
        mb.reset();
        return;
    }

    match frame[1] {
        0x03 => handle_read(usart1, frame),
        0x06 => handle_write_single(usart1, frame),
        0x10 => handle_write_multi(usart1, frame),
        _ => {}
    }

    mb.reset();
}

/// ---------------------------------------------------------------------------
/// 0x03 Read
/// ---------------------------------------------------------------------------
fn handle_read(usart1: &pac::USART1, frame: &[u8]) {
    let start = u16::from_be_bytes([frame[2], frame[3]]) as usize;
    let count = u16::from_be_bytes([frame[4], frame[5]]) as usize;

    if count == 0 || start + count > REG_COUNT {
        return;
    }

    let regs = unsafe { &*HOLDING_REGS.0.get() };

    let mut resp = [0u8; 256];

    resp[0] = SLAVE_ADDR;
    resp[1] = 0x03;
    resp[2] = (count * 2) as u8;

    for i in 0..count {
        let val = regs[start + i];
        resp[3 + i * 2] = (val >> 8) as u8;
        resp[4 + i * 2] = val as u8;
    }

    let len = 3 + count * 2;

    let crc = crc16(&resp[..len]);
    resp[len] = crc as u8;
    resp[len + 1] = (crc >> 8) as u8;

    usart1::write_buf(usart1, &resp[..len + 2]);
}

/// ---------------------------------------------------------------------------
/// 0x06 Write Single
/// ---------------------------------------------------------------------------
fn handle_write_single(usart1: &pac::USART1, frame: &[u8]) {
    let addr = u16::from_be_bytes([frame[2], frame[3]]) as usize;
    let value = u16::from_be_bytes([frame[4], frame[5]]);

    if addr < REG_COUNT {
        unsafe {
            (*HOLDING_REGS.0.get())[addr] = value;
        }

        usart1::write_buf(usart1, &frame[..8]);
    }
}

/// ---------------------------------------------------------------------------
/// 0x10 Write Multiple
/// ---------------------------------------------------------------------------
fn handle_write_multi(usart1: &pac::USART1, frame: &[u8]) {
    let start = u16::from_be_bytes([frame[2], frame[3]]) as usize;
    let count = u16::from_be_bytes([frame[4], frame[5]]) as usize;

    if count == 0 || start + count > REG_COUNT {
        return;
    }

    unsafe {
        let regs = &mut *HOLDING_REGS.0.get();

        for i in 0..count {
            let hi = frame[7 + i * 2];
            let lo = frame[8 + i * 2];
            regs[start + i] = u16::from_be_bytes([hi, lo]);
        }
    }

    let mut resp = [0u8; 8];

    resp[0] = SLAVE_ADDR;
    resp[1] = 0x10;
    resp[2] = frame[2];
    resp[3] = frame[3];
    resp[4] = frame[4];
    resp[5] = frame[5];

    let crc = crc16(&resp[..6]);
    resp[6] = crc as u8;
    resp[7] = (crc >> 8) as u8;

    usart1::write_buf(usart1, &resp);
}
