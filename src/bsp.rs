use stm32c0::stm32c031 as pac;

/// ---------------------------------------------------------------------------
/// 🧩 Board Support Package (BSP)
/// ---------------------------------------------------------------------------
///
/// Stateless hardware bring-up layer.
///
/// ---------------------------------------------------------------------------
/// 🧠 DESIGN PHILOSOPHY
/// ---------------------------------------------------------------------------
///
/// - BSP performs **one-time hardware configuration only**
/// - BSP is **stateless** (no ownership, no global state)
/// - BSP does NOT contain any application logic
/// - BSP does NOT depend on higher-level modules (like encoder)
///
/// ---------------------------------------------------------------------------
/// ⚙️ RESPONSIBILITIES
/// ---------------------------------------------------------------------------
///
/// - GPIO configuration (mode, pull, speed)
/// - EXTI configuration (edge trigger, masking)
/// - Peripheral clock enable (minimal and explicit)
///
/// ---------------------------------------------------------------------------
/// ⚠️ NON-RESPONSIBILITIES
/// ---------------------------------------------------------------------------
///
/// - No ISR logic
/// - No peripheral abstraction
/// - No runtime decisions
///
/// ---------------------------------------------------------------------------
/// 🔗 COUPLING WITH encoder.rs
/// ---------------------------------------------------------------------------
///
/// This BSP is tightly aligned with `encoder.rs` assumptions:
///
/// - ENC_A → PA0 (EXTI0)
/// - ENC_B → PA1 (EXTI1)
/// - ENC_Z → PA2 (EXTI2)
///
/// - All signals are on SAME GPIO port → enables atomic IDR read
/// - EXTI configured for BOTH edges → required for X4 decoding
///
/// ---------------------------------------------------------------------------

/// ---------------------------------------------------------------------------
/// GPIOA Initialization
/// ---------------------------------------------------------------------------
///
/// Configures encoder input pins:
/// - PA0 → ENC_A
/// - PA1 → ENC_B
/// - PA2 → ENC_Z
///
/// ---------------------------------------------------------------------------
/// 🧠 Design Rationale
/// ---------------------------------------------------------------------------
///
/// - Input mode ensures no interference with external encoder driver
/// - Floating input assumes encoder provides push-pull or line driver
/// - Same port (GPIOA) enables atomic sampling in ISR
///
pub fn init_gpioa(gpioa: &pac::GPIOA) {
    // ---------------------------------------------------------------------
    // 1. MODE CONFIGURATION
    // ---------------------------------------------------------------------
    //
    // Set PA0, PA1, PA2 → INPUT mode
    //
    // This disables:
    // - output drivers (prevents contention)
    // - analog mode leakage paths
    //
    gpioa.moder().modify(|_, w| {
        w.mode0().input();
        w.mode1().input();
        w.mode2().input()
    });

    // ---------------------------------------------------------------------
    // 2. PULL-UP / PULL-DOWN CONFIGURATION
    // ---------------------------------------------------------------------
    //
    // Floating inputs assume:
    // - external encoder actively drives signals
    //
    // If encoder is open-collector/open-drain:
    // → change to pull-up
    //
    gpioa.pupdr().modify(|_, w| {
        w.pupd0().floating();
        w.pupd1().floating();
        w.pupd2().floating()
    });

    // ---------------------------------------------------------------------
    // 3. OUTPUT SPEED CONFIGURATION
    // ---------------------------------------------------------------------
    //
    // Not functionally required for inputs,
    // but explicitly set for deterministic register state.
    //
    gpioa.ospeedr().modify(|_, w| {
        w.ospeed0().low_speed();
        w.ospeed1().low_speed();
        w.ospeed2().low_speed()
    });
}

/// ---------------------------------------------------------------------------
/// EXTI Initialization
/// ---------------------------------------------------------------------------
///
/// Configures external interrupts for:
/// - PA0 → EXTI0 (ENC_A)
/// - PA1 → EXTI1 (ENC_B)
/// - PA2 → EXTI2 (ENC_Z)
///
/// ---------------------------------------------------------------------------
/// 🧠 Design Rationale
/// ---------------------------------------------------------------------------
///
/// - BOTH edges enabled → required for X4 quadrature decoding
/// - No filtering at EXTI level → ISR handles validity via LUT
/// - All lines enabled → shared IRQ handler processes them uniformly
///
/// ---------------------------------------------------------------------------
/// ⚠️ STM32C0 Specific Behavior
/// ---------------------------------------------------------------------------
///
/// - Rising edges set RPR1
/// - Falling edges set FPR1
/// - BOTH must be cleared in ISR
///
pub fn init_exti(exti: &pac::EXTI) {
    // ---------------------------------------------------------------------
    // 1. RISING EDGE CONFIGURATION
    // ---------------------------------------------------------------------
    //
    // Trigger on LOW → HIGH transitions
    //
    exti.rtsr1().modify(|_, w| {
        w.rt0().set_bit();
        w.rt1().set_bit();
        w.rt2().set_bit()
    });

    // ---------------------------------------------------------------------
    // 2. FALLING EDGE CONFIGURATION
    // ---------------------------------------------------------------------
    //
    // Trigger on HIGH → LOW transitions
    //
    exti.ftsr1().modify(|_, w| {
        w.ft0().set_bit();
        w.ft1().set_bit();
        w.ft2().set_bit()
    });

    // ---------------------------------------------------------------------
    // 3. CLEAR PENDING FLAGS
    // ---------------------------------------------------------------------
    //
    // Ensures no stale interrupt is pending before enabling
    //
    // Without this:
    // → ISR may fire immediately after unmask
    //
    exti.rpr1().write(|w| {
        w.rpif0().set_bit();
        w.rpif1().set_bit();
        w.rpif2().set_bit()
    });

    // Ensure write completes before proceeding
    cortex_m::asm::dsb();
    cortex_m::asm::isb();

    // ---------------------------------------------------------------------
    // 4. INTERRUPT MASK (ENABLE)
    // ---------------------------------------------------------------------
    //
    // Unmask EXTI lines 0,1,2
    //
    // After this:
    // → events propagate to NVIC
    //
    exti.imr1().write(|w| unsafe { w.bits(0b111) });
}

/// ---------------------------------------------------------------------------
/// Clock Initialization
/// ---------------------------------------------------------------------------
///
/// Enables only required peripheral clocks.
///
/// ---------------------------------------------------------------------------
/// 🧠 Design Rationale
/// ---------------------------------------------------------------------------
///
/// - Minimal clock enable → reduces power and side effects
/// - Explicit enable avoids hidden dependencies
///
pub fn init_clocks(rcc: &pac::RCC) {
    // ---------------------------------------------------------------------
    // 1. GPIOA CLOCK ENABLE
    // ---------------------------------------------------------------------
    //
    // Required for:
    // - reading IDR in encoder ISR
    //
    rcc.iopenr().modify(|_, w| w.gpioaen().set_bit());

    // ---------------------------------------------------------------------
    // 2. SYSCFG CLOCK ENABLE
    // ---------------------------------------------------------------------
    //
    // Required for:
    // - EXTI line routing (port selection)
    //
    rcc.apbenr2().modify(|_, w| w.syscfgen().set_bit());

    // Ensure clock is active before peripheral access
    cortex_m::asm::dsb();
}