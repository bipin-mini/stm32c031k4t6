#![no_std]
#![no_main]

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
///
mod bsp;
mod encoder;
mod flash;
mod modbus;
mod relay;
mod tm1638;
mod usart1;

use panic_halt as _;

use bsp::SYSCLK_HZ;
use rtic::app;
use stm32c0::stm32c031 as pac;
use systick_monotonic::*;

#[app(device = pac, peripherals = true, dispatchers = [I2C, SPI, ADC])]
mod app {

    use super::*;

    // -----------------------------------------------------------------
    // MONOTONIC TIMER (SysTick @ 1 kHz)
    // -----------------------------------------------------------------
    //
    // Provides RTIC time base:
    // - 1 tick = 1 ms
    // - Backed by SysTick
    // - Clock source = SYSCLK_HZ (48 MHz)
    //
    #[monotonic(binds = SysTick, default = true)]
    type SysMono = Systick<1000>;

    // -----------------------------------------------------------------
    // SHARED RESOURCES
    // -----------------------------------------------------------------
    //
    // Accessed from:
    // - power_fail_irq (writer)
    // - background task (reader)
    //
    // Design:
    // - Minimal shared state
    // - No contention in ISR hot paths
    //
    #[shared]
    struct Shared {
        power_fail_flag: bool,
    }

    // -----------------------------------------------------------------
    // LOCAL RESOURCES
    // -----------------------------------------------------------------
    //
    // Owned by specific tasks/ISRs
    // No locking required
    //
    #[local]
    struct Local {
        usart1: pac::USART1,
    }

    // -----------------------------------------------------------------
    // SYSTEM INITIALIZATION
    // -----------------------------------------------------------------
    //
    // Responsibilities:
    // - Configure system clock (48 MHz)
    // - Initialize GPIO and EXTI
    // - Initialize encoder and USART
    // - Start monotonic timer
    // - Spawn background task
    //
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

        // USART pins (PA9/PA10)
        bsp::init_usart1_pins(&gpioa);

        // RS485 DE/RE (PA3)
        bsp::init_rs485_de(&gpioa);

        // -------------------------------------------------------------
        // EXTI CONFIGURATION
        // -------------------------------------------------------------
        bsp::init_exti(&exti);

        // -------------------------------------------------------------
        // SUBSYSTEM INITIALIZATION
        // -------------------------------------------------------------
        encoder::init();
        usart1::init(&usart1_dev, &rcc);
        relay::init(&gpiob);
        // -------------------------------------------------------------
        // MONOTONIC TIMER
        // -------------------------------------------------------------
        let mono = Systick::new(ctx.core.SYST, SYSCLK_HZ);

        // -------------------------------------------------------------
        // BACKGROUND TASK START
        // -------------------------------------------------------------
        //
        // NOTE:
        // Currently acts as:
        // - Modbus RX drain
        // - Placeholder for main control loop
        //
        // TODO:
        // Replace with deterministic 1 kHz control loop
        //
        blink::spawn().ok();

        (
            Shared {
                power_fail_flag: false,
            },
            Local { usart1: usart1_dev },
            init::Monotonics(mono),
        )
    }

    // -----------------------------------------------------------------
    // ENCODER ISR (EXTI0_1 → PA0, PA1)
    // -----------------------------------------------------------------
    //
    // Responsibilities:
    // - Perform quadrature decode
    // - Update pulse counter
    //
    // Constraints:
    // - Constant-time execution
    // - No branching
    // - No shared resource access
    //
    #[task(binds = EXTI0_1, priority = 2)]
    fn exti0_1(_ctx: exti0_1::Context) {
        encoder::isr();
    }

    // -----------------------------------------------------------------
    // INDEX ISR (EXTI2 → PA2)
    // -----------------------------------------------------------------
    //
    // STATUS: TBD
    //
    // Requirement:
    // - Latch encoder position on index pulse
    // - Must NOT modify main pulse counter
    //
    // Current behavior:
    // - Interrupt is acknowledged and cleared
    // - Event is ignored
    //
    // Safety:
    // - Prevents interrupt lockup
    // - No impact on encoder counting path
    //
    #[task(binds = EXTI2_3, priority = 2)]
    fn exti2_3(_ctx: exti2_3::Context) {
        let exti = unsafe { &*pac::EXTI::ptr() };

        const MASK: u32 = 1 << 2; // EXTI2 (PA2)

        exti.rpr1().write(|w| unsafe { w.bits(MASK) });
        exti.fpr1().write(|w| unsafe { w.bits(MASK) });
    }

    // -----------------------------------------------------------------
    // USART1 ISR
    // -----------------------------------------------------------------
    //
    // Responsibilities:
    // - Receive byte (RXFNE)
    // - Push to lock-free buffer
    //
    // Constraints:
    // - Constant-time
    // - No blocking
    // - No shared resource access
    //
    #[task(binds = USART1, priority = 1, local = [usart1])]
    fn usart1_irq(ctx: usart1_irq::Context) {
        usart1::isr(ctx.local.usart1);
    }

    // -----------------------------------------------------------------
    // BACKGROUND TASK (SYSTEM LOOP - TEMPORARY)
    // -----------------------------------------------------------------
    //
    // CURRENT ROLE:
    // - Drain USART RX buffer
    //
    // LIMITATIONS:
    // - Runs at 500 ms → NOT compliant with system requirements
    //
    // REQUIRED (per spec):
    // - ≥ 1 kHz execution rate
    // - Power-fail handling
    // - Modbus processing
    // - Scaling / display / relay logic
    //
    // STATUS:
    // - Placeholder only (TBD)
    //
    #[task]
    fn blink(_ctx: blink::Context) {
        while let Some(_b) = usart1::read() {
            // future: modbus::process_byte(_b);
        }

        blink::spawn_after(500.millis()).ok();
    }

    // -----------------------------------------------------------------
    // POWER-FAIL ISR (EXTI6 → PA6)
    // -----------------------------------------------------------------
    //
    // Priority: Highest in system
    //
    // Responsibilities:
    // - Detect power loss (falling edge)
    // - Set flag for deferred handling
    //
    // Constraints:
    // - No flash writes here
    // - No blocking operations
    // - Minimal execution time
    //
    // Deferred handling must:
    // - Disable interrupts
    // - Save state to flash
    // - Turn off relays
    // - Halt system
    //
    #[task(binds = EXTI4_15, priority = 3, shared = [power_fail_flag])]
    fn power_fail_irq(mut ctx: power_fail_irq::Context) {
        ctx.shared.power_fail_flag.lock(|flag| {
            *flag = true;
        });

        // Clear EXTI6 flags
        let exti = unsafe { &*pac::EXTI::ptr() };
        const MASK: u32 = 1 << 6;

        exti.rpr1().write(|w| unsafe { w.bits(MASK) });
        exti.fpr1().write(|w| unsafe { w.bits(MASK) });
    }
}
