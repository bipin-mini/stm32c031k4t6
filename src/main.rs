#![no_std]
#![no_main]

/// ---------------------------------------------------------------------
/// APPLICATION MODULE STRUCTURE
/// ---------------------------------------------------------------------
///
/// Target MCU: STM32C031
///
/// This is an RTIC-based real-time firmware.
///
/// ---------------------------------------------------------------------
/// 🧠 SYSTEM PURPOSE
/// ---------------------------------------------------------------------
///
/// - High-speed quadrature encoder acquisition (interrupt-driven)
/// - Deterministic real-time processing
/// - Foundation for industrial DRO system
///
/// ---------------------------------------------------------------------
/// 🏗️ ARCHITECTURE
/// ---------------------------------------------------------------------
///
/// - RTIC owns all peripherals (single ownership model)
/// - BSP performs hardware bring-up only (stateless)
/// - encoder.rs handles ONLY ISR-level decoding
/// - Main loop / tasks handle scaling, UI, communication
///
/// ---------------------------------------------------------------------
/// ⚙️ DESIGN GOALS
/// ---------------------------------------------------------------------
///
/// - Deterministic execution (cycle-bounded ISR)
/// - Zero dynamic allocation
/// - Interrupt-safe architecture
/// - Clear separation of responsibilities
///
mod bsp;
mod encoder;

use panic_halt as _;

use rtic::app;
use stm32c0::stm32c031 as pac;
use systick_monotonic::*;

#[app(device = pac, peripherals = true, dispatchers = [I2C, SPI, ADC])]
mod app {

    use super::*;

    /// ---------------------------------------------------------------------
    /// MONOTONIC TIMER (SysTick आधारित RTIC timebase)
    /// ---------------------------------------------------------------------
    ///
    /// Provides:
    /// - task scheduling
    /// - delays
    /// - periodic execution
    ///
    /// ⚠️ MUST match actual system clock
    ///
    #[monotonic(binds = SysTick, default = true)]
    type SysMono = Systick<1000>;

    /// ---------------------------------------------------------------------
    /// SHARED RESOURCES
    /// ---------------------------------------------------------------------
    ///
    /// Currently unused.
    ///
    /// Future:
    /// - encoder value (if synchronized access required)
    /// - Modbus buffers
    ///
    #[shared]
    struct Shared {}

    /// ---------------------------------------------------------------------
    /// LOCAL RESOURCES
    /// ---------------------------------------------------------------------
    ///
    /// Task-local (non-shared) state.
    ///
    #[local]
    struct Local {}

    /// ---------------------------------------------------------------------
    /// SYSTEM INITIALIZATION
    /// ---------------------------------------------------------------------
    ///
    /// Execution order is **strict and intentional**:
    ///
    /// 1. Enable clocks
    /// 2. Configure GPIO
    /// 3. Configure EXTI
    /// 4. Initialize encoder state
    /// 5. Start RTIC scheduling
    ///
    #[init]
    fn init(ctx: init::Context) -> (Shared, Local, init::Monotonics) {

        // -----------------------------------------------------------------
        // PERIPHERAL OWNERSHIP (RTIC)
        // -----------------------------------------------------------------
        let dp = ctx.device;

        let gpioa = dp.GPIOA;
        let exti  = dp.EXTI;
        let rcc   = dp.RCC;

        // -----------------------------------------------------------------
        // BSP: CLOCK ENABLE
        // -----------------------------------------------------------------
        //
        // Required before accessing GPIO / SYSCFG / EXTI
        //
        bsp::init_clocks(&rcc);

        // -----------------------------------------------------------------
        // BSP: GPIO CONFIGURATION
        // -----------------------------------------------------------------
        //
        // Configures:
        // PA0 → ENC_A
        // PA1 → ENC_B
        // PA2 → ENC_Z
        //
        bsp::init_gpioa(&gpioa);

        // -----------------------------------------------------------------
        // BSP: EXTI CONFIGURATION
        // -----------------------------------------------------------------
        //
        // Enables:
        // - both edge detection
        // - EXTI0,1,2 lines
        //
        bsp::init_exti(&exti);

        // -----------------------------------------------------------------
        // ENCODER INITIALIZATION
        // -----------------------------------------------------------------
        //
        // MUST be done AFTER GPIO setup
        //
        // Purpose:
        // - capture initial A/B state
        // - avoid false first transition
        //
        encoder::init();

        // -----------------------------------------------------------------
        // MONOTONIC TIMER INIT
        // -----------------------------------------------------------------
        //
        // NOTE:
        // Currently assumes 16 MHz system clock (default HSI)
        //
        let mono = Systick::new(ctx.core.SYST, 16_000_000);

        // -----------------------------------------------------------------
        // START SYSTEM TASKS
        // -----------------------------------------------------------------
        blink::spawn().ok();

        (Shared {}, Local {}, init::Monotonics(mono))
    }

    /// ---------------------------------------------------------------------
    /// EXTI INTERRUPT HANDLER (ENCODER CORE)
    /// ---------------------------------------------------------------------
    ///
    /// Handles:
    /// - EXTI0 → ENC_A (PA0)
    /// - EXTI1 → ENC_B (PA1)
    ///
    /// ⚠️ CRITICAL DESIGN:
    ///
    /// - This ISR must be **highest priority (after power-fail)**
    /// - Must execute in bounded time (~50–60 cycles)
    /// - Must NOT perform any non-deterministic work
    ///
    /// Delegates ALL logic to encoder.rs
    ///
    #[task(binds = EXTI0_1, priority = 2)]
    fn exti0_1(_ctx: exti0_1::Context) {
        encoder::isr();
    }

    /// ---------------------------------------------------------------------
    /// EXTI2 HANDLER (INDEX PULSE - FUTURE)
    /// ---------------------------------------------------------------------
    ///
    /// Reserved for:
    /// - ENC_Z (index pulse)
    ///
    /// Currently not implemented.
    ///
    #[task(binds = EXTI2_3, priority = 2)]
    fn exti2_3(_ctx: exti2_3::Context) {
        // Future:
        // encoder::index_isr();
    }

    /// ---------------------------------------------------------------------
    /// PERIODIC TASK: SYSTEM HEARTBEAT
    /// ---------------------------------------------------------------------
    ///
    /// Purpose:
    /// - validate RTIC scheduling
    /// - placeholder for watchdog feed
    ///
    #[task]
    fn blink(_ctx: blink::Context) {
        // Re-schedule periodically
        blink::spawn_after(500.millis()).ok();
    }
}