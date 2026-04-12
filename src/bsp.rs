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
/// - BSP is **stateless (no ownership, no global state)**
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
/// 🧩 EXTI ROUTING NOTE (STM32C0 / v0.16.0 PAC)
/// ---------------------------------------------------------------------------
///
/// On this device and PAC version:
///
/// - EXTI line-to-pin mapping is FIXED at reset for GPIOA (PA0–PA15)
/// - No SYSCFG_EXTICR register is exposed in this PAC
/// - Therefore EXTI0/1/2 are implicitly mapped to PA0/PA1/PA2
///
/// ✔ This BSP intentionally relies on this fixed hardware behavior.
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
/// - Floating input assumes encoder provides push-pull output stage
///   (if open-collector, use pull-ups instead)
/// - Same port (GPIOA) enables atomic sampling in ISR
///
pub fn init_gpioa(gpioa: &pac::GPIOA) {
    // MODE CONFIGURATION
    gpioa.moder().modify(|_, w| {
        w.mode0().input();
        w.mode1().input();
        w.mode2().input()
    });

    // PULL-UP / PULL-DOWN CONFIGURATION
    gpioa.pupdr().modify(|_, w| {
        w.pupd0().floating();
        w.pupd1().floating();
        w.pupd2().floating()
    });

    // OUTPUT SPEED CONFIGURATION
    // Not functionally required for inputs; kept only for deterministic reset state.
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
/// - BOTH must be cleared in ISR or during init
///
pub fn init_exti(exti: &pac::EXTI) {
    // RISING EDGE CONFIGURATION
    exti.rtsr1().modify(|_, w| {
        w.rt0().set_bit();
        w.rt1().set_bit();
        w.rt2().set_bit()
    });

    // FALLING EDGE CONFIGURATION
    exti.ftsr1().modify(|_, w| {
        w.ft0().set_bit();
        w.ft1().set_bit();
        w.ft2().set_bit()
    });

    // CLEAR PENDING FLAGS (RISING)
    exti.rpr1().write(|w| {
        w.rpif0().set_bit();
        w.rpif1().set_bit();
        w.rpif2().set_bit()
    });

    // CLEAR PENDING FLAGS (FALLING)
    exti.fpr1().write(|w| {
        w.fpif0().set_bit();
        w.fpif1().set_bit();
        w.fpif2().set_bit()
    });

    cortex_m::asm::dsb();
    cortex_m::asm::isb();

    // INTERRUPT MASK (ENABLE)
    //
    // NOTE:
    // STM32C0 PAC exposes IMR1 as raw bitfield register.
    // Therefore we must use raw bits(), not field accessors.
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
    // GPIOA CLOCK ENABLE
    rcc.iopenr().modify(|_, w| w.gpioaen().set_bit());

    // SYSCFG CLOCK ENABLE (kept for completeness; EXTI routing is fixed in this PAC)
    rcc.apbenr2().modify(|_, w| w.syscfgen().set_bit());

    cortex_m::asm::dsb();
}