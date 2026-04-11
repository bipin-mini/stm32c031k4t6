#![no_std]
#![no_main]

/// ---------------------------------------------------------------------
/// APPLICATION MODULE STRUCTURE
/// ---------------------------------------------------------------------
/// This is an RTIC-based real-time firmware for STM32C031.
///
/// SYSTEM PURPOSE:
/// - High-speed quadrature encoder acquisition (future module)
/// - Deterministic interrupt-driven processing
/// - Low-latency embedded control system foundation
///
/// ARCHITECTURE:
/// - RTIC owns all peripherals (single ownership model)
/// - BSP performs hardware bring-up only
/// - Tasks handle runtime logic
///
/// DESIGN GOALS:
/// - deterministic execution
/// - ISR-safe architecture
/// - zero dynamic allocation
/// - predictable timing behavior
mod bsp;

use panic_halt as _;

use rtic::app;
use stm32c0::stm32c031 as pac;
use systick_monotonic::*;

#[app(device = pac, peripherals = true, dispatchers = [I2C, SPI, ADC])]
mod app {

    use super::*;

    /// ---------------------------------------------------------------------
    /// MONOTONIC TIMER
    /// ---------------------------------------------------------------------
    /// Provides RTIC timebase for:
    /// - scheduling tasks
    /// - periodic execution (blink, sampling, control loops)
    /// - future encoder sampling synchronization
    ///
    /// NOTE:
    /// Frequency MUST match actual system clock configuration.
    #[monotonic(binds = SysTick, default = true)]
    type SysMono = Systick<1000>;

    /// ---------------------------------------------------------------------
    /// SHARED RESOURCES
    /// ---------------------------------------------------------------------
    /// Resources accessed across multiple tasks or interrupts.
    ///
    /// Currently empty, but will later include:
    /// - encoder count
    /// - scaling factor
    /// - Modbus buffers
    #[shared]
    struct Shared {}

    /// ---------------------------------------------------------------------
    /// LOCAL RESOURCES
    /// ---------------------------------------------------------------------
    /// Task-local state (not shared between tasks/interrupts).
    ///
    /// Typical future uses:
    /// - debounced button state
    /// - temporary buffers
    /// - ISR shadow variables
    #[local]
    struct Local {}

    /// ---------------------------------------------------------------------
    /// SYSTEM INITIALIZATION ENTRY POINT
    /// ---------------------------------------------------------------------
    ///
    /// This function runs once at boot.
    ///
    /// RESPONSIBILITIES:
    /// 1. Extract PAC peripherals (single ownership)
    /// 2. Perform hardware bring-up via BSP
    /// 3. Initialize system timer (monotonic clock)
    /// 4. Start first scheduled tasks
    ///
    /// ORDER IS CRITICAL:
    /// Clock → GPIO → EXTI → RTIC tasks
    #[init]
    fn init(ctx: init::Context) -> (Shared, Local, init::Monotonics) {

        // -----------------------------------------------------------------
        // PERIPHERAL OWNERSHIP ACQUISITION
        // -----------------------------------------------------------------
        // RTIC provides full ownership of device peripherals here.
        // These are split manually and passed into BSP.
        let dp = ctx.device;

        let gpioa = dp.GPIOA;
        let exti = dp.EXTI;
        let rcc = dp.RCC;

        // -----------------------------------------------------------------
        // HARDWARE BRING-UP (BSP LAYER)
        // -----------------------------------------------------------------
        // BSP configures hardware registers deterministically.
        // It does NOT store state or take ownership.
        //
        // CLOCK CONFIGURATION:
        // Enables required peripheral clocks
        bsp::init_clocks(&rcc);

        // GPIO CONFIGURATION:
        // Configures encoder pins PA0–PA2
        bsp::init_gpioa(&gpioa);

        // EXTI CONFIGURATION:
        // Enables edge-triggered interrupt system for encoder signals
        bsp::init_exti(&exti);

        // -----------------------------------------------------------------
        // MONOTONIC TIMER INITIALIZATION
        // -----------------------------------------------------------------
        // SysTick is configured as RTIC scheduling backbone.
        // Frequency must match actual CPU clock.
        let mono = Systick::new(ctx.core.SYST, 16_000_000);

        // -----------------------------------------------------------------
        // STARTUP TASK SCHEDULING
        // -----------------------------------------------------------------
        // Kick off periodic system task.
        //
        // This is non-blocking and safe failure is ignored here
        // (startup phase, system not yet loaded).
        blink::spawn().ok();

        // -----------------------------------------------------------------
        // RETURN RTIC RESOURCES
        // -----------------------------------------------------------------
        // Shared + Local resources are initialized here.
        // Monotonics returned to RTIC scheduler.
        (Shared {}, Local {}, init::Monotonics(mono))
    }

    /// ---------------------------------------------------------------------
    /// PERIODIC TASK: BLINK (PLACEHOLDER SYSTEM HEARTBEAT)
    /// ---------------------------------------------------------------------
    ///
    /// PURPOSE:
    /// - validate RTIC scheduling
    /// - verify monotonic timer functionality
    /// - act as placeholder for system heartbeat
    ///
    /// FUTURE USE:
    /// - status LED blink
    /// - system alive indicator
    /// - watchdog feed point
    #[task]
    fn blink(_ctx: blink::Context) {

        // -----------------------------------------------------------------
        // PERIODIC RESCHEDULING
        // -----------------------------------------------------------------
        // Re-schedules itself every 500 ms.
        //
        // NOTE:
        // In production systems:
        // - failure to schedule should be logged
        // - not silently ignored
        blink::spawn_after(500.millis()).ok();
    }
}