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
///     • Used for short frames (Modbus RTU)
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
///     • Modbus → protocol layer
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
/// - `static mut` is avoided (Rust 2024 compliance)
/// - Access is strictly partitioned:
///     • ISR → write only
///     • Task → read only
///
/// Invariants:
/// - No aliasing of mutable references
/// - No concurrent access to same field
///
/// This pattern is standard for lock-free embedded SPSC buffers.
/// ---------------------------------------------------------------------------

const RX_BUF_SIZE: usize = 256; // Must be power-of-two

// ---------------------------------------------------------------------------
// RX Ring Buffer (Lock-Free SPSC)
// ---------------------------------------------------------------------------
struct RingBuffer {
    buf: [u8; RX_BUF_SIZE],
    head: usize, // write index (ISR only)
    tail: usize, // read index (task only)
}

impl RingBuffer {
    // Create empty buffer
    const fn new() -> Self {
        Self {
            buf: [0; RX_BUF_SIZE],
            head: 0,
            tail: 0,
        }
    }

    // Push byte (ISR context)
    //
    // Properties:
    // - Constant time
    // - No branching except overflow check
    // - No blocking
    //
    // Behavior:
    // - On overflow → byte is dropped (per HLD requirement)
    #[inline(always)]
    fn push(&mut self, byte: u8) {
        let next = (self.head + 1) & (RX_BUF_SIZE - 1);

        if next != self.tail {
            self.buf[self.head] = byte;
            self.head = next;
        }
        // Overflow condition intentionally ignored
    }

    // Pop byte (task context)
    //
    // Returns:
    // - Some(byte) → data available
    // - None       → buffer empty
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
// Global RX Buffer Wrapper
// ---------------------------------------------------------------------------
//
// Encapsulates UnsafeCell to provide controlled interior mutability.
//
// SAFETY INVARIANTS:
// - Only ISR writes (push)
// - Only task reads (pop)
// - No concurrent mutation of same index
//
struct RxBuf(UnsafeCell<RingBuffer>);

unsafe impl Sync for RxBuf {}

static RX_BUF: RxBuf = RxBuf(UnsafeCell::new(RingBuffer::new()));

// ---------------------------------------------------------------------------
// USART1 Initialization
// ---------------------------------------------------------------------------
//
// Configures:
// - 9600 baud
// - 8 data bits, no parity, 1 stop bit (8N1)
// - RX interrupt enabled
//
// Assumptions:
// - GPIO already configured via BSP
// - System clock = 16 MHz
//
// NOTE:
// If system clock changes, BRR must be recalculated.
//
pub fn init(usart1: &pac::USART1, rcc: &pac::RCC) {
    // Enable USART1 peripheral clock
    rcc.apbenr2().modify(|_, w| w.usart1en().set_bit());

    // Disable USART before configuration
    usart1.cr1().modify(|_, w| w.ue().clear_bit());

    // Configure baud rate (16 MHz / 9600 ≈ 1667)
    usart1.brr().write(|w| unsafe { w.bits(1667) });

    // Enable RX, TX and RX interrupt
    usart1.cr1().modify(|_, w| {
        w.re().set_bit();
        w.te().set_bit();
        w.rxneie().set_bit()
    });

    // Enable USART peripheral
    usart1.cr1().modify(|_, w| w.ue().set_bit());
}

// ---------------------------------------------------------------------------
// USART1 Interrupt Handler
// ---------------------------------------------------------------------------
//
// Responsibilities:
// - Read received byte (RXFNE)
// - Handle overrun error (ORE)
//
// Constraints:
// - Constant execution time
// - No loops
// - No blocking
//
// Must be called from RTIC:
// #[task(binds = USART1)]
//
#[inline(always)]
pub fn isr(usart1: &pac::USART1) {
    let isr = usart1.isr().read();

    // RXFNE: data available in RDR
    if isr.rxfne().bit_is_set() {
        let byte = usart1.rdr().read().bits() as u8;

        unsafe {
            (*RX_BUF.0.get()).push(byte);
        }
    }

    // ORE: overrun error
    //
    // Occurs if RDR not read in time.
    // Cleared by reading RDR.
    if isr.ore().bit_is_set() {
        let _ = usart1.rdr().read().bits();
    }
}

// ---------------------------------------------------------------------------
// Non-blocking Read
// ---------------------------------------------------------------------------
//
// Called from task context only.
//
// Returns:
// - Some(byte) → data available
// - None       → buffer empty
//
#[inline(always)]
pub fn read() -> Option<u8> {
    unsafe { (*RX_BUF.0.get()).pop() }
}

// ---------------------------------------------------------------------------
// Blocking Write (Single Byte)
// ---------------------------------------------------------------------------
//
// Waits until TX register is ready, then writes byte.
//
// Deterministic and bounded.
//
#[inline(always)]
pub fn write(usart1: &pac::USART1, byte: u8) {
    while usart1.isr().read().txfe().bit_is_clear() {}

    usart1.tdr().write(|w| unsafe { w.bits(byte as u32) });
}

// ---------------------------------------------------------------------------
// Blocking Write (Buffer)
// ---------------------------------------------------------------------------
//
// Sequence:
// 1. Enable RS485 transmitter (DE HIGH)
// 2. Transmit all bytes
// 3. Wait for TC (last bit shifted out)
// 4. Disable transmitter (DE LOW)
//
// TC ensures:
// - Shift register empty
// - Line idle → safe to release bus
//
pub fn write_buf(usart1: &pac::USART1, buf: &[u8]) {
    // Enter TX mode (RS485)
    tx_start();

    // Small guard delay (driver enable time)
    cortex_m::asm::nop();
    cortex_m::asm::nop();

    // Send bytes
    for &b in buf {
        write(usart1, b);
    }

    // Wait until transmission fully complete
    while usart1.isr().read().tc().bit_is_clear() {}

    // Return to RX mode
    tx_end();
}

// ---------------------------------------------------------------------------
// RS485 DE CONTROL (PA3)
// ---------------------------------------------------------------------------
//
// Direct register access used for:
// - Minimal latency
// - Deterministic timing
//
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
// RS485 Direction Control API
// ---------------------------------------------------------------------------
//
// tx_start():
// - Enables transmitter (DE HIGH)
//
// tx_end():
// - Returns to receive mode (DE LOW)
//
// Timing:
// - Must assert before first byte
// - Must release after TC
//
#[inline(always)]
pub fn tx_start() {
    de_high();
}

#[inline(always)]
pub fn tx_end() {
    de_low();
}
