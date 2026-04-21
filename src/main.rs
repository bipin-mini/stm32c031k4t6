#![no_std]
#![no_main]

use panic_halt as _;

/* =====================================================================
   PRE-INIT HOOK (RAM FUNCTION RELOCATION)
   =====================================================================

   STM32 Flash cannot be read while it is being programmed or erased.
   Therefore, all Flash-modifying routines must execute from RAM.

   The linker script places such functions in a special section:
       `.ramfunc`

   This pre-init hook runs *before main()* and copies that section
   from Flash → SRAM.

   Symbols:
     _sramfunc : start of source (Flash)
     _eramfunc : end of source (Flash)
     _ramfunc  : destination (RAM)

   This is mandatory for safe Flash operations.
*/
#[no_mangle]
pub unsafe extern "C" fn __pre_init() {
    extern "C" {
        static mut _sramfunc: u32;
        static mut _eramfunc: u32;
        static mut _ramfunc: u32;
    }

    let mut src = core::ptr::addr_of!(_sramfunc);
    let mut dst = core::ptr::addr_of_mut!(_ramfunc);
    let end = core::ptr::addr_of!(_eramfunc);

    while src < end {
        core::ptr::write(dst, core::ptr::read(src));
        src = src.add(1);
        dst = dst.add(1);
    }
}

/* =====================================================================
   MODULE ORGANIZATION
   =====================================================================

   bsp      → board support (clock, GPIO, EXTI setup)
   flash    → low-level STM32 Flash driver (no policy)
   eeprom   → log-structured storage layer on Flash
   drivers  → hardware drivers (encoder, UART, relay, display)
*/
mod bsp;
pub mod flash;
mod modbus;

mod drivers {
    pub mod encoder;
    pub mod relay;
    pub mod tm1638;
    pub mod uart;
}

mod storage {
    pub mod eeprom;
}

use bsp::SYSCLK_HZ;
use rtic::app;
use stm32c0::stm32c031 as pac;
use systick_monotonic::*;

/* =====================================================================
   POWER FAIL SNAPSHOT (CRITICAL STATE BUFFER)
   =====================================================================

   This structure captures the *minimal required system state*
   at the exact moment of power failure.

   Design goals:
   - Must be trivially copyable (no heap, no allocation)
   - Must be written extremely fast (ISR-safe)
   - Must survive until idle loop commits to Flash

   Fields:
   - encoder : last known encoder count
   - valid   : indicates snapshot is pending commit

   NOTE:
   This acts as a "bridge" between ISR context and main execution.
*/
#[derive(Copy, Clone)]
pub struct PowerSnapshot {
    encoder: u64,
    valid: bool,
}

/* =====================================================================
   RTIC APPLICATION
   =====================================================================

   RTIC (Real-Time Interrupt-driven Concurrency) ensures:
   - Safe shared access without data races
   - Deterministic interrupt priority handling
   - Zero-cost abstractions (no runtime scheduler)
*/
#[app(device = pac, peripherals = true, dispatchers = [I2C, SPI, ADC])]
mod app {

    use super::*;
    use crate::flash::Stm32Flash;
    use crate::storage::eeprom::Eeprom;

    /* ---------------------------------------------------------
       MONOTONIC TIMER (1 kHz SYSTEM TICK)
       ---------------------------------------------------------

       Used for timekeeping and scheduling if needed.
       Currently not heavily used, but provides future extensibility.
    */
    #[monotonic(binds = SysTick, default = true)]
    type SysMono = Systick<1000>;

    /* ---------------------------------------------------------
       SHARED STATE (MULTI-CONTEXT ACCESS)
       ---------------------------------------------------------

       Shared between:
       - Interrupt context (power_fail_irq)
       - Idle loop (main execution)

       Access must be protected using `.lock()`.
    */
    #[shared]
    struct Shared {
        /// Power-fail snapshot shared between ISR and idle loop
        snapshot: PowerSnapshot,
    }

    /* ---------------------------------------------------------
       LOCAL STATE (OWNED BY A SINGLE CONTEXT)
       ---------------------------------------------------------

       These resources are only accessed from the idle context,
       so no locking is required.
    */
    #[local]
    struct Local {
        uart: pac::USART1,
        eeprom: Eeprom,
        gpiob: pac::GPIOB,
    }

    /* =============================================================
       SYSTEM INITIALIZATION
       ============================================================= */
    #[init]
    fn init(ctx: init::Context) -> (Shared, Local, init::Monotonics) {
        let dp = ctx.device;

        // ---------------------------------------------------------
        // BOARD INITIALIZATION
        // ---------------------------------------------------------
        bsp::init_clocks(&dp.RCC);
        bsp::init_gpioa(&dp.GPIOA);
        bsp::init_usart1_pins(&dp.GPIOA);
        bsp::init_rs485_de(&dp.GPIOA);
        bsp::init_exti(&dp.EXTI);

        // ---------------------------------------------------------
        // DRIVER INITIALIZATION
        // ---------------------------------------------------------
        drivers::encoder::init();
        drivers::uart::init(&dp.USART1, &dp.RCC);
        drivers::relay::init(&dp.GPIOB);
        drivers::tm1638::init();

        // ---------------------------------------------------------
        // EEPROM (FLASH BACKED STORAGE)
        // ---------------------------------------------------------
        let flash = Stm32Flash::new(dp.FLASH);
        let eeprom = Eeprom::new(flash);

        // ---------------------------------------------------------
        // SYSTEM TIMER
        // ---------------------------------------------------------
        let mono = Systick::new(ctx.core.SYST, SYSCLK_HZ);

        (
            Shared {
                snapshot: PowerSnapshot {
                    encoder: 0,
                    valid: false,
                },
            },
            Local {
                uart: dp.USART1,
                eeprom,
                gpiob: dp.GPIOB,
            },
            init::Monotonics(mono),
        )
    }

    /* =============================================================
       ENCODER INTERRUPT
       =============================================================

       Handles quadrature decoding (X4 mode typically).
       High priority to avoid missed pulses.
    */
    #[task(binds = EXTI0_1, priority = 2)]
    fn exti0_1(_ctx: exti0_1::Context) {
        drivers::encoder::isr();
    }

    /* =============================================================
       POWER FAIL INTERRUPT (CRITICAL PATH)
       =============================================================

       This is the MOST critical part of the system.

       Design constraints:
       - Must execute in minimal time
       - Must NOT perform Flash writes here
       - Must capture system state deterministically

       Strategy:
       1. Disable interrupts (freeze system)
       2. Capture encoder state immediately
       3. Mark snapshot as valid
       4. Exit quickly

       Flash write is deferred to idle loop.
    */
    #[task(binds = EXTI4_15, priority = 3, shared = [snapshot])]
    fn power_fail_irq(mut ctx: power_fail_irq::Context) {

        // ---------------------------------------------------------
        // HARD STOP: prevent further system activity
        // ---------------------------------------------------------
        cortex_m::interrupt::disable();

        // ---------------------------------------------------------
        // CAPTURE CRITICAL STATE
        // ---------------------------------------------------------
        let encoder = drivers::encoder::get_count() as u64;

        ctx.shared.snapshot.lock(|snap| {
            snap.encoder = encoder;
            snap.valid = true;
        });

        // ---------------------------------------------------------
        // CLEAR EXTI INTERRUPT FLAG
        // ---------------------------------------------------------
        let exti = unsafe { &*pac::EXTI::ptr() };
        const MASK: u32 = 1 << 6;

        exti.rpr1().write(|w| unsafe { w.bits(MASK) });
        exti.fpr1().write(|w| unsafe { w.bits(MASK) });
    }

    /* =============================================================
       UART INTERRUPT
       ============================================================= */
    #[task(binds = USART1, priority = 1, local = [uart])]
    fn usart1_irq(ctx: usart1_irq::Context) {
        drivers::uart::isr(ctx.local.uart);
    }

    /* =============================================================
       IDLE LOOP (MAIN EXECUTION CONTEXT)
       =============================================================

       Responsibilities:
       - Handle power-fail commit
       - Process background tasks
       - Enter low-power wait when idle

       IMPORTANT:
       This is the ONLY place where Flash write occurs during
       power-fail handling.
    */
    #[idle(shared = [snapshot], local = [eeprom, gpiob])]
    fn idle(mut ctx: idle::Context) -> ! {

        loop {

            // -----------------------------------------------------
            // POWER FAIL COMMIT (EXECUTED EXACTLY ONCE)
            // -----------------------------------------------------
            let mut do_commit = None;

            ctx.shared.snapshot.lock(|snap| {
                if snap.valid {
                    do_commit = Some(snap.encoder);
                    snap.valid = false; // consume event
                }
            });

            if let Some(encoder) = do_commit {

                // -------------------------------------------------
                // EEPROM WRITE
                // -------------------------------------------------
                // Assumption:
                // - At least ONE slot is always free
                // - No erase happens in this path
                let _ = ctx.local.eeprom.write(0x01, encoder);

                // -------------------------------------------------
                // SAFE OUTPUT SHUTDOWN
                // -------------------------------------------------
                drivers::relay::off(ctx.local.gpiob);

                // -------------------------------------------------
                // HALT SYSTEM (WAIT FOR POWER LOSS)
                // -------------------------------------------------
                loop {
                    cortex_m::asm::nop();
                }
            }

            // -----------------------------------------------------
            // BACKGROUND PROCESSING
            // -----------------------------------------------------
            while let Some(_b) = drivers::uart::read() {}

            // -----------------------------------------------------
            // LOW POWER WAIT
            // -----------------------------------------------------
            cortex_m::asm::wfi();
        }
    }
}