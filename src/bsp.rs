use stm32c0::stm32c031 as pac;

/// Board Support Package (BSP)
///
/// Stateless hardware bring-up layer.
/// All functions are deterministic and non-owning.
///
/// Responsibilities:
/// - GPIO configuration
/// - EXTI configuration
/// - Clock enable (only if strictly required)
///
/// NOTE:
/// BSP NEVER owns peripherals.
/// BSP NEVER returns state.
pub fn init_gpioa(gpioa: &pac::GPIOA) {
    // -----------------------------
    // 1. MODE CONFIGURATION
    // -----------------------------
    // Set PA0, PA1, PA2 to INPUT mode.
    //
    // This disables:
    // - output drivers
    // - analog mode leakage paths
    gpioa.moder().modify(|_, w| {
        w.mode0().input();
        w.mode1().input();
        w.mode2().input()
    });

    // -----------------------------
    // 2. PULL-UP / PULL-DOWN CONFIG
    // -----------------------------
    // Defines default logic level when encoder is idle.
    //
    // floating = external push-pull encoder assumed
    gpioa.pupdr().modify(|_, w| {
        w.pupd0().floating();
        w.pupd1().floating();
        w.pupd2().floating()
    });

    // -----------------------------
    // 3. OUTPUT SPEED REGISTER
    // -----------------------------
    // Not functionally relevant for inputs, but explicitly set
    // for deterministic reset state.
    gpioa.ospeedr().modify(|_, w| {
        w.ospeed0().low_speed();
        w.ospeed1().low_speed();
        w.ospeed2().low_speed()
    });
}

pub fn init_exti(exti: &pac::EXTI) {
    // -----------------------------
    // 1. RISING EDGE CONFIGURATION
    // -----------------------------
    // Trigger on LOW → HIGH transitions
    exti.rtsr1().modify(|_, w| {
        w.rt0().set_bit();
        w.rt1().set_bit();
        w.rt2().set_bit()
    });

    // -----------------------------
    // 2. FALLING EDGE CONFIGURATION
    // -----------------------------
    // Trigger on HIGH → LOW transitions
    exti.ftsr1().modify(|_, w| {
        w.ft0().set_bit();
        w.ft1().set_bit();
        w.ft2().set_bit()
    });

    // -----------------------------
    // 3. CLEAR PENDING INTERRUPTS
    // -----------------------------
    // Prevent immediate ISR firing after enable
    exti.rpr1().write(|w| {
        w.rpif0().set_bit();
        w.rpif1().set_bit();
        w.rpif2().set_bit()
    });

    cortex_m::asm::dsb();
    cortex_m::asm::isb();

    // -----------------------------
    // 4. ENABLE INTERRUPTS
    // -----------------------------
    // Unmask EXTI0–2 lines
    exti.imr1().write(|w| unsafe { w.bits(0b111) });
}

pub fn init_clocks(rcc: &pac::RCC) {
    // Enable GPIOA clock
    rcc.iopenr().modify(|_, w| w.gpioaen().set_bit());

    // Enable SYSCFG clock
    rcc.apbenr2().modify(|_, w| w.syscfgen().set_bit());

    cortex_m::asm::dsb();
}