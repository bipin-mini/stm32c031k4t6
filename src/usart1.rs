use core::cell::UnsafeCell;
use stm32c0::stm32c031 as pac;

/// ---------------------------------------------------------------------------
/// USART1 Driver (Interrupt-driven RX, Polling TX)
/// ---------------------------------------------------------------------------
///
/// Peripheral Mapping:
/// - USART1
/// - TX → PA9  (AF1)
/// - RX → PA10 (AF1)
///
/// ---------------------------------------------------------------------------
/// 🧠 Functional Overview
/// ---------------------------------------------------------------------------
///
/// - RX path:
///     • Interrupt-driven (RXFNE event)
///     • Lock-free buffering (SPSC ring buffer)
///
/// - TX path:
///     • Polling-based (blocking, deterministic)
///     • Used for short frames (e.g., Modbus RTU)
///
/// - RS485:
///     • DE pin controlled in software (PA3)
///     • Direction switching handled in TX path
///
/// ---------------------------------------------------------------------------
/// 🧠 Design Principles
/// ---------------------------------------------------------------------------
///
/// - ISR executes in constant time (single-byte enqueue)
/// - No dynamic allocation
/// - No locks or critical sections
/// - Zero-copy RX path
/// - Strict separation:
///     • Driver → transport only
///     • Higher layer → protocol handling
///
/// ---------------------------------------------------------------------------
/// ⚠️ Concurrency Model (SPSC - Single Producer / Single Consumer)
/// ---------------------------------------------------------------------------
///
/// - Producer:
///     • USART1 ISR (push)
///
/// - Consumer:
///     • RTIC task / main loop (pop)
///
/// Guarantees:
/// - Single-core Cortex-M0+
/// - No concurrent modification of same index
/// - Head modified only by ISR
/// - Tail modified only by task
///
/// ---------------------------------------------------------------------------
/// ⚠️ Safety Model
/// ---------------------------------------------------------------------------
///
/// Uses `UnsafeCell` to enable interior mutability of a static buffer.
///
/// Justification:
/// - `static mut` avoided (Rust 2024 compliance)
/// - Access partitioned:
///     • ISR → write only
///     • Task → read only
///
/// Invariants:
/// - No aliasing of mutable references
/// - No concurrent access to same field
///
/// Standard pattern for lock-free embedded SPSC buffers.
/// ---------------------------------------------------------------------------

const RX_BUF_SIZE: usize = 256; // Must be power-of-two

// ---------------------------------------------------------------------------
// RX Ring Buffer (Lock-Free SPSC)
// ---------------------------------------------------------------------------
struct RingBuffer {
    buf: [u8; RX_BUF_SIZE],
    head: usize, // ISR only
    tail: usize, // task only
}

impl RingBuffer {
    const fn new() -> Self {
        Self {
            buf: [0; RX_BUF_SIZE],
            head: 0,
            tail: 0,
        }
    }

    #[inline(always)]
    fn push(&mut self, byte: u8) {
        let next = (self.head + 1) & (RX_BUF_SIZE - 1);

        if next != self.tail {
            self.buf[self.head] = byte;
            self.head = next;
        }
        // overflow → drop byte
    }

    #[inline(always)]
    fn pop(&mut self) -> Option<u8> {
        if self.head == self.tail {
            None
        } else {
            let b = self.buf[self.tail];
            self.tail = (self.tail + 1) & (RX_BUF_SIZE - 1);
            Some(b)
        }
    }
}

// ---------------------------------------------------------------------------
// Global RX Buffer
// ---------------------------------------------------------------------------
struct RxBuf(UnsafeCell<RingBuffer>);

unsafe impl Sync for RxBuf {}

static RX_BUF: RxBuf = RxBuf(UnsafeCell::new(RingBuffer::new()));

// ---------------------------------------------------------------------------
// USART1 Initialization
// ---------------------------------------------------------------------------
//
// Configuration:
// - Baud rate: 9600
// - Frame: 8 data bits, no parity, 1 stop bit (8N1)
// - RX interrupt enabled
//
// Assumptions:
// - System clock = 48 MHz (HSI48 / 1 via BSP)
// - Oversampling = 16 (default)
//
// Baud calculation:
//     BRR = fCK / baud = 48_000_000 / 9600 = 5000
//
pub fn init(usart1: &pac::USART1, rcc: &pac::RCC) {
    // Enable USART1 clock
    rcc.apbenr2().modify(|_, w| w.usart1en().set_bit());

    // Disable USART before config
    usart1.cr1().modify(|_, w| w.ue().clear_bit());

    // Set baud rate for 48 MHz clock
    usart1.brr().write(|w| unsafe { w.bits(5000) });

    // Enable RX, TX, RX interrupt
    usart1.cr1().modify(|_, w| {
        w.re().set_bit();
        w.te().set_bit();
        w.rxneie().set_bit()
    });

    // Enable USART
    usart1.cr1().modify(|_, w| w.ue().set_bit());
}

// ---------------------------------------------------------------------------
// USART1 Interrupt Handler
// ---------------------------------------------------------------------------
#[inline(always)]
pub fn isr(usart1: &pac::USART1) {
    let isr = usart1.isr().read();

    // RXFNE → byte available
    if isr.rxfne().bit_is_set() {
        let byte = usart1.rdr().read().bits() as u8;

        unsafe {
            (*RX_BUF.0.get()).push(byte);
        }
    }

    // ORE → overrun
    if isr.ore().bit_is_set() {
        let _ = usart1.rdr().read().bits();
    }
}

// ---------------------------------------------------------------------------
// Non-blocking Read
// ---------------------------------------------------------------------------
#[inline(always)]
pub fn read() -> Option<u8> {
    unsafe { (*RX_BUF.0.get()).pop() }
}

// ---------------------------------------------------------------------------
// Blocking Write (Single Byte)
// ---------------------------------------------------------------------------
#[inline(always)]
pub fn write(usart1: &pac::USART1, byte: u8) {
    while usart1.isr().read().txfe().bit_is_clear() {}

    usart1.tdr().write(|w| unsafe { w.bits(byte as u32) });
}

// ---------------------------------------------------------------------------
// Blocking Write (Buffer)
// ---------------------------------------------------------------------------
pub fn write_buf(usart1: &pac::USART1, buf: &[u8]) {
    tx_start();

    // DE setup delay (very small, depends on transceiver)
    cortex_m::asm::nop();
    cortex_m::asm::nop();

    for &b in buf {
        write(usart1, b);
    }

    // Wait for full transmission (shift register empty)
    while usart1.isr().read().tc().bit_is_clear() {}

    tx_end();
}

// ---------------------------------------------------------------------------
// RS485 DE Control (PA3)
// ---------------------------------------------------------------------------
#[inline(always)]
fn de_high() {
    let gpioa = unsafe { &*pac::GPIOA::ptr() };
    gpioa.bsrr().write(|w| w.bs3().set_bit());
}

#[inline(always)]
fn de_low() {
    let gpioa = unsafe { &*pac::GPIOA::ptr() };
    gpioa.bsrr().write(|w| w.br3().set_bit());
}

// ---------------------------------------------------------------------------
// RS485 Direction API
// ---------------------------------------------------------------------------
#[inline(always)]
pub fn tx_start() {
    de_high();
}

#[inline(always)]
pub fn tx_end() {
    de_low();
}
