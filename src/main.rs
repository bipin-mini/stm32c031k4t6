#![no_std]
#![no_main]


use panic_halt as _;

/* ---------------------------------------------------------
   PRE-INIT HOOK
--------------------------------------------------------- */
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

/// ---------------------------------------------------------------------
/// APPLICATION MODULE STRUCTURE
/// ---------------------------------------------------------------------
///
/// Top-level firmware composition:
///
/// - bsp      → hardware bring-up (clocks, GPIO, EXTI)
/// - encoder  → real-time quadrature decoding (ISR only)
/// - modbus   → protocol layer (TBD)
/// - usart1   → transport layer (interrupt RX, polling TX)
/// - flash    → low-level Flash driver (STM32C0 specific)
/// - eeprom   → AN4894-based EEPROM emulation (Flash-backed)
///
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


#[app(device = pac, peripherals = true, dispatchers = [I2C, SPI, ADC])]
mod app {

    use super::*;

    // -----------------------------------------------------------------
    // EEPROM / FLASH IMPORTS (LINKAGE + VALIDATION)
    // -----------------------------------------------------------------
    //
    // PURPOSE:
    // - Ensure Flash + EEPROM modules are part of final binary
    // - Validate integration at compile-time
    //
    // NOTE:
    // - EEPROM logic is initialized but not yet used by application
    // - Full AN4894 behavior is implemented in module
    //
    use crate::flash::Stm32Flash;
    use crate::storage::eeprom::Eeprom;

    // -----------------------------------------------------------------
    // MONOTONIC TIMER (SysTick @ 1 kHz)
    // -----------------------------------------------------------------
    #[monotonic(binds = SysTick, default = true)]
    type SysMono = Systick<1000>;

    // -----------------------------------------------------------------
    // SHARED RESOURCES
    // -----------------------------------------------------------------
    //
    // Access pattern:
    // - power_fail_irq → sets flag (fast, constant-time)
    // - idle           → consumes flag (executes shutdown sequence)
    //
    // DESIGN INTENT:
    // - ISR remains minimal and deterministic
    // - All heavy operations are deferred
    //
    #[shared]
    struct Shared {
        power_fail_flag: bool,
    }

    // -----------------------------------------------------------------
    // LOCAL RESOURCES
    // -----------------------------------------------------------------
    #[local]
    struct Local {
        uart: pac::USART1,

        /// -----------------------------------------------------------------
        /// EEPROM (FLASH-BACKED STORAGE)
        /// -----------------------------------------------------------------
        ///
        /// IMPLEMENTATION:
        /// - Based on ST AN4894 (dual-page log structure)
        /// - Supports power-loss recovery at algorithm level
        ///
        /// CURRENT USAGE:
        /// - Initialized at boot
        /// - Not actively used by application yet
        ///
        /// IMPORTANT SYSTEM CONTRACT:
        ///
        /// NORMAL OPERATION:
        /// - Full EEPROM API may be used
        /// - Page transfers and erase operations are allowed
        ///
        /// POWER-FAIL CONDITION:
        /// - ONLY bounded Flash program operations are allowed
        /// - Page erase MUST NOT be triggered
        /// - Page transfer MUST NOT be triggered
        ///
        /// EMERGENCY WRITE CONSTRAINT:
        /// - Application MUST guarantee:
        ///     - Write does NOT trigger page transfer
        ///     - Sufficient free space exists in active page
        ///
        /// SYSTEM-LEVEL REQUIREMENTS:
        /// - Early power-fail detection (EXTI6 ✔)
        /// - Sufficient hold-up time (external capacitor REQUIRED)
        /// - Deterministic shutdown path (see idle task)
        ///
        /// FUTURE ROLE:
        /// - Store configuration (Modbus, calibration)
        /// - Persist machine/encoder state
        ///
        eeprom: Eeprom,
    }

    // -----------------------------------------------------------------
    // SYSTEM INITIALIZATION
    // -----------------------------------------------------------------
    #[init]
    fn init(ctx: init::Context) -> (Shared, Local, init::Monotonics) {
        let dp = ctx.device;

        let gpioa = dp.GPIOA;
        let gpiob = dp.GPIOB;
        let exti = dp.EXTI;
        let rcc = dp.RCC;
        let usart1_dev = dp.USART1;

        // -------------------------------------------------------------
        // CLOCK CONFIGURATION (48 MHz SYSCLK)
        // -------------------------------------------------------------
        bsp::init_clocks(&rcc);

        // -------------------------------------------------------------
        // GPIO CONFIGURATION
        // -------------------------------------------------------------
        bsp::init_gpioa(&gpioa);
        bsp::init_usart1_pins(&gpioa);
        bsp::init_rs485_de(&gpioa);

        // -------------------------------------------------------------
        // EXTI CONFIGURATION
        // -------------------------------------------------------------
        bsp::init_exti(&exti);

        // -------------------------------------------------------------
        // SUBSYSTEM INITIALIZATION
        // -------------------------------------------------------------
        drivers::encoder::init();
        drivers::uart::init(&usart1_dev, &rcc);
        drivers::relay::init(&gpiob);
        drivers::tm1638::init();
        drivers::relay::off(&gpiob);

        // -------------------------------------------------------------
        // EEPROM INITIALIZATION
        // -------------------------------------------------------------
        //
        // BEHAVIOR:
        // - Evaluates Flash page states
        // - Recovers interrupted transfers (AN4894)
        // - Performs formatting if required
        //
        // IMPORTANT:
        // - This is the ONLY phase where erase/transfer is allowed
        // - Must NEVER be triggered during power-fail handling
        //
        let flash = Stm32Flash::new(dp.FLASH);
        let eeprom = Eeprom::new(flash).unwrap();

        // -------------------------------------------------------------
        // MONOTONIC TIMER
        // -------------------------------------------------------------
        let mono = Systick::new(ctx.core.SYST, SYSCLK_HZ);

        (
            Shared {
                power_fail_flag: false,
            },
            Local {
                uart: usart1_dev,
                eeprom,
            },
            init::Monotonics(mono),
        )
    }

    // -----------------------------------------------------------------
    // ENCODER ISR
    // -----------------------------------------------------------------
    #[task(binds = EXTI0_1, priority = 2)]
    fn exti0_1(_ctx: exti0_1::Context) {
        drivers::encoder::isr();
    }

    // -----------------------------------------------------------------
    // INDEX ISR
    // -----------------------------------------------------------------
    #[task(binds = EXTI2_3, priority = 2)]
    fn exti2_3(_ctx: exti2_3::Context) {
        let exti = unsafe { &*pac::EXTI::ptr() };

        const MASK: u32 = 1 << 2;

        exti.rpr1().write(|w| unsafe { w.bits(MASK) });
        exti.fpr1().write(|w| unsafe { w.bits(MASK) });
    }

    // -----------------------------------------------------------------
    // USART1 ISR
    // -----------------------------------------------------------------
    #[task(binds = USART1, priority = 1, local = [uart])]
    fn usart1_irq(ctx: usart1_irq::Context) {
        drivers::uart::isr(ctx.local.uart);
    }

    // -----------------------------------------------------------------
    // IDLE LOOP (NORMAL + EMERGENCY EXECUTION CONTEXT)
    // -----------------------------------------------------------------
    //
    // ROLE:
    // - Low-priority processing
    // - Deferred system actions
    //
    // POWER-FAIL MODEL:
    //
    // 1. ISR (EXTI6) → sets flag + disables interrupts
    // 2. idle()      → executes shutdown with full CPU control
    //
    // CRITICAL GUARANTEE:
    // - When power_fail_flag is observed:
    //     → Interrupts are already globally disabled
    //     → No preemption is possible
    //     → Execution is fully deterministic
    //
    #[idle(shared = [power_fail_flag], local = [eeprom])]
    fn idle(mut ctx: idle::Context) -> ! {
        loop {
            // ---------------------------------------------------------
            // POWER FAIL CHECK
            // ---------------------------------------------------------
            let mut power_fail = false;

            ctx.shared.power_fail_flag.lock(|flag| {
                if *flag {
                    *flag = false;
                    power_fail = true;
                }
            });

            if power_fail {
                // -----------------------------------------------------
                // CRITICAL SHUTDOWN SEQUENCE (DETERMINISTIC)
                // -----------------------------------------------------
                //
                // EXECUTION CONTEXT:
                // - Interrupts already disabled
                // - No preemption possible
                // - Full CPU ownership
                //
                // REQUIRED (future):
                //
                // drivers::relay::off(...);
                // eeprom.write_power_fail(...);
                // loop {}
                //
                // CONSTRAINTS:
                // - Must NOT perform page erase
                // - Must NOT trigger page transfer
                // - Only bounded Flash writes allowed
                // - Must complete within hold-up time
                //
                loop {
                    cortex_m::asm::nop();
                }
            }

            // ---------------------------------------------------------
            // NORMAL BACKGROUND WORK
            // ---------------------------------------------------------
            while let Some(_b) = drivers::uart::read() {}

            cortex_m::asm::wfi();
        }
    }

    // -----------------------------------------------------------------
    // POWER-FAIL ISR (EXTI6)
    // -----------------------------------------------------------------
    //
    // DESIGN PRINCIPLE:
    // - Minimal and constant-time
    // - No Flash operations
    //
    // RESPONSIBILITY:
    // - Detect early power loss
    // - Signal system via flag
    //
    // CRITICAL BEHAVIOR:
    // - Global interrupts are disabled before exit
    // - Guarantees:
    //     → No pending interrupts will execute
    //     → CPU transitions directly to idle()
    //     → System enters deterministic emergency mode
    //
    #[task(binds = EXTI4_15, priority = 3, shared = [power_fail_flag])]
    fn power_fail_irq(mut ctx: power_fail_irq::Context) {
        // Set flag
        ctx.shared.power_fail_flag.lock(|flag| {
            *flag = true;
        });

        // Clear EXTI
        let exti = unsafe { &*pac::EXTI::ptr() };
        const MASK: u32 = 1 << 6;
        exti.rpr1().write(|w| unsafe { w.bits(MASK) });
        exti.fpr1().write(|w| unsafe { w.bits(MASK) });

        // Disable all interrupts → enter deterministic mode
        cortex_m::interrupt::disable();
    }
}
