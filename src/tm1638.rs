#![allow(dead_code)]

use stm32c0::stm32c031 as pac;

/// ---------------------------------------------------------------------------
/// TM1638 Driver (Bit-Banged, Deterministic, Task-Level Only)
/// ---------------------------------------------------------------------------
///
/// Hardware Interface (GPIOA):
/// - PA4 → STB  (strobe / chip select, active LOW)
/// - PA5 → CLK  (serial clock)
/// - PA7 → DIO  (bidirectional data)
///
/// ---------------------------------------------------------------------------
/// 🧠 DESIGN OBJECTIVES
/// ---------------------------------------------------------------------------
///
/// - Fully deterministic execution (fixed cycles per byte)
/// - No dynamic allocation
/// - No blocking delays (only bounded NOP-based timing)
/// - No ISR usage (task-context only)
/// - Direct register access (no HAL, no abstraction overhead)
///
/// ---------------------------------------------------------------------------
/// ⚠️ EXECUTION MODEL
/// ---------------------------------------------------------------------------
///
/// - Intended for **main loop / RTIC task context only**
/// - Must NEVER be called from:
///     • Encoder ISR
///     • Power-fail ISR
///
/// - Timing is CPU-dependent:
///     • Calibrated for 48 MHz SYSCLK
///
/// ---------------------------------------------------------------------------
/// ⚠️ ELECTRICAL / PROTOCOL NOTES
/// ---------------------------------------------------------------------------
///
/// - TM1638 uses LSB-first serial protocol
/// - Data is latched on CLK rising edge
/// - STB LOW → transaction active
/// - STB HIGH → transaction end
///
/// - DIO is half-duplex:
///     • Output during write
///     • Input during read
///
/// ---------------------------------------------------------------------------

// Pin masks (GPIOA)
const STB: u32 = 1 << 4;
const CLK: u32 = 1 << 5;
const DIO: u32 = 1 << 7;

// ---------------------------------------------------------------------------
// LOW-LEVEL GPIO ACCESS (DIRECT REGISTER BLOCK)
// ---------------------------------------------------------------------------

type GpioaRb = pac::gpioa::RegisterBlock;

/// Returns raw GPIOA register block
///
/// SAFETY:
/// - Single-core system
/// - No aliasing with mutable references
#[inline(always)]
fn gpio() -> &'static GpioaRb {
    unsafe { &*pac::GPIOA::ptr() }
}

// ---------------------------------------------------------------------------
// PIN CONTROL (BSRR → atomic, single-cycle)
// ---------------------------------------------------------------------------

#[inline(always)]
fn stb_high() {
    gpio().bsrr().write(|w| w.bs4().set_bit());
}

#[inline(always)]
fn stb_low() {
    gpio().bsrr().write(|w| w.br4().set_bit());
}

#[inline(always)]
fn clk_high() {
    gpio().bsrr().write(|w| w.bs5().set_bit());
}

#[inline(always)]
fn clk_low() {
    gpio().bsrr().write(|w| w.br5().set_bit());
}

#[inline(always)]
fn dio_high() {
    gpio().bsrr().write(|w| w.bs7().set_bit());
}

#[inline(always)]
fn dio_low() {
    gpio().bsrr().write(|w| w.br7().set_bit());
}

/// Read DIO input level
#[inline(always)]
fn dio_read() -> bool {
    (gpio().idr().read().bits() & DIO) != 0
}

// ---------------------------------------------------------------------------
// DIO DIRECTION CONTROL
// ---------------------------------------------------------------------------
///
/// ⚠️ NOTE:
/// - Uses MODER read-modify-write (non-constant-time)
/// - Acceptable because:
///     • Not used in ISR
///     • TM1638 is low-speed peripheral
///
#[inline(always)]
fn dio_output() {
    gpio().moder().modify(|_, w| w.mode7().output());
}

#[inline(always)]
fn dio_input() {
    gpio().moder().modify(|_, w| w.mode7().input());
}

// ---------------------------------------------------------------------------
// TIMING CONTROL
// ---------------------------------------------------------------------------
///
/// Minimal delay to satisfy TM1638 timing:
/// - Provides setup/hold margin
/// - Fixed instruction count → deterministic
///
#[inline(always)]
fn delay() {
    cortex_m::asm::nop();
    cortex_m::asm::nop();
}

// ---------------------------------------------------------------------------
// BYTE TRANSFER (LSB FIRST)
// ---------------------------------------------------------------------------
///
/// - Exactly 8 iterations
/// - No data-dependent branching except bit test
/// - Constant execution path
///
#[inline(always)]
fn write_byte(mut data: u8) {
    dio_output();

    for _ in 0..8 {
        clk_low();

        if (data & 0x01) != 0 {
            dio_high();
        } else {
            dio_low();
        }

        delay();

        clk_high();
        delay();

        data >>= 1;
    }
}

/// Read one byte (LSB first)
#[inline(always)]
fn read_byte() -> u8 {
    dio_input();

    let mut data = 0u8;

    for i in 0..8 {
        clk_low();
        delay();

        if dio_read() {
            data |= 1 << i;
        }

        clk_high();
        delay();
    }

    data
}

// ---------------------------------------------------------------------------
// TM1638 COMMAND SET
// ---------------------------------------------------------------------------

const CMD_DATA_AUTO_INC: u8 = 0x40;
const CMD_DATA_READ: u8 = 0x42;
const CMD_ADDR: u8 = 0xC0;
const CMD_DISPLAY_ON: u8 = 0x88; // OR with brightness (0–7)

// ---------------------------------------------------------------------------
// PUBLIC API
// ---------------------------------------------------------------------------

/// Initialize TM1638 interface
///
/// - Sets idle bus state
/// - Enables display with maximum brightness
pub fn init() {
    // Idle state
    stb_high();
    clk_high();
    dio_high();

    set_display(true, 7);
}

/// Configure display ON/OFF and brightness
///
/// brightness:
/// - Range: 0–7
pub fn set_display(on: bool, brightness: u8) {
    let cmd = if on {
        CMD_DISPLAY_ON | (brightness & 0x07)
    } else {
        0x80 // display OFF
    };

    stb_low();
    write_byte(cmd);
    stb_high();
}

/// Write full display RAM (16 bytes)
///
/// Memory layout:
/// - Even addresses → segment data
/// - Odd addresses  → LED control
///
/// Sequence:
/// 1. Set auto-increment mode
/// 2. Write starting address
/// 3. Stream 16 bytes
pub fn write_display(data: &[u8; 16]) {
    // Data command
    stb_low();
    write_byte(CMD_DATA_AUTO_INC);
    stb_high();

    // Address command
    stb_low();
    write_byte(CMD_ADDR);

    for &b in data {
        write_byte(b);
    }

    stb_high();
}

/// Clear display (all segments OFF)
pub fn clear() {
    let buf = [0u8; 16];
    write_display(&buf);
}

/// Read key scan data (4 bytes)
///
/// Each byte contains multiplexed key states.
/// Decoding is handled at higher layer.
pub fn read_keys(buf: &mut [u8; 4]) {
    stb_low();
    write_byte(CMD_DATA_READ);

    for b in buf.iter_mut() {
        *b = read_byte();
    }

    stb_high();
}