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
/// Note:
/// EXTI3 is also mapped to PA3 via EXTICR1 reset state, but is not used.
///
/// ---------------------------------------------------------------------------
/// 🧠 Design Rationale
/// ---------------------------------------------------------------------------
///
/// - Both-edge triggering is enabled for X4 quadrature decoding
/// - No hardware filtering is used; signal validation is handled in ISR
///   using software (e.g., LUT/state machine)
/// - EXTI lines are shared and handled in a unified interrupt handler
///
/// ---------------------------------------------------------------------------
/// ⚠️ STM32C0 Specific Behavior (STM32C031)
/// ---------------------------------------------------------------------------
///
/// - Rising edges set RPR1 pending bits
/// - Falling edges set FPR1 pending bits
/// - Both rising and falling pending flags must be cleared in software
///   during initialization and/or in the ISR
///
/// - EXTI routing is configured via EXTICR1 (default reset state maps
///   all used lines to GPIOA in this configuration)
///
pub fn init_exti(exti: &pac::EXTI) {
    // PA0–PA3 mapping (reset default: all to GPIOA)
    exti.exticr1().write(|w| unsafe { w.bits(0x0000) });

    // -----------------------------------------------------------------------
    // Rising edge configuration (trigger on rising transitions)
    // -----------------------------------------------------------------------
    exti.rtsr1().modify(|_, w| {
        w.rt0().set_bit();
        w.rt1().set_bit();
        w.rt2().set_bit()
    });

    // -----------------------------------------------------------------------
    // Falling edge configuration (trigger on falling transitions)
    // -----------------------------------------------------------------------
    exti.ftsr1().modify(|_, w| {
        w.ft0().set_bit();
        w.ft1().set_bit();
        w.ft2().set_bit()
    });

    // -----------------------------------------------------------------------
    // Clear pending flags (rising edge events)
    // -----------------------------------------------------------------------
    exti.rpr1().write(|w| {
        w.rpif0().set_bit();
        w.rpif1().set_bit();
        w.rpif2().set_bit()
    });

    // -----------------------------------------------------------------------
    // Clear pending flags (falling edge events)
    // -----------------------------------------------------------------------
    exti.fpr1().write(|w| {
        w.fpif0().set_bit();
        w.fpif1().set_bit();
        w.fpif2().set_bit()
    });

    cortex_m::asm::dsb();
    cortex_m::asm::isb();

    // -----------------------------------------------------------------------
    // Interrupt mask enable
    //
    // Enables EXTI lines:
    // - EXTI0 (PA0)
    // - EXTI1 (PA1)
    // - EXTI2 (PA2)
    // -----------------------------------------------------------------------
    exti.imr1()
        .modify(|r, w| unsafe { w.bits(r.bits() | 0b111) });

    // -----------------------------------------------------------------------
    // NVIC enable
    //
    // Enables interrupt groups:
    // - EXTI0_1 → handles EXTI0 and EXTI1
    // - EXTI2_3 → handles EXTI2 and EXTI3
    // -----------------------------------------------------------------------
    unsafe {
        cortex_m::peripheral::NVIC::unmask(pac::Interrupt::EXTI0_1);
        cortex_m::peripheral::NVIC::unmask(pac::Interrupt::EXTI2_3);
    }
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

/// ---------------------------------------------------------------------------
/// USART1 GPIO Pin Initialization
/// ---------------------------------------------------------------------------
///
/// Configures GPIOA pins for USART1 peripheral:
///
/// - PA9  → USART1_TX (AF1)
/// - PA10 → USART1_RX (AF1)
///
/// ---------------------------------------------------------------------------
/// 🧠 Design Rationale
/// ---------------------------------------------------------------------------
///
/// - Alternate Function mode is required to connect GPIO to USART peripheral
/// - AF1 is the correct mapping for USART1 on STM32C031
/// - TX pin is configured as high-speed to ensure clean signal edges
/// - RX pin remains floating (external driver must define level)
///
/// ---------------------------------------------------------------------------
/// ⚠️ STM32C0 PAC (v0.16.0) Specific Behavior
/// ---------------------------------------------------------------------------
///
/// - AFRH register uses indexed access: `afr(n)`
/// - No field helpers like `afrh9()` exist
/// - Writing AF requires raw `bits()` → requires `unsafe`
///
/// AFRH layout (4 bits per pin):
///
/// | Pin  | AFR Index | Bit Range |
/// |------|----------|-----------|
/// | PA8  | afr(0)   | [3:0]     |
/// | PA9  | afr(1)   | [7:4]     |
/// | PA10 | afr(2)   | [11:8]    |
///
/// ---------------------------------------------------------------------------
/// ⚠️ Safety
/// ---------------------------------------------------------------------------
///
/// - `bits()` is unsafe because it bypasses type-level validation
/// - Safe here because:
///   - AF1 is valid for USART1
///   - Bit positions are hardware-defined and fixed
///
/// ---------------------------------------------------------------------------
/// ⚙️ Electrical Assumptions
/// ---------------------------------------------------------------------------
///
/// - TX (PA9) → push-pull output from MCU
/// - RX (PA10) → driven by external device (RS485 transceiver, etc.)
/// - If RX line is open/floating externally, pull-up/down must be added
///
/// ---------------------------------------------------------------------------
pub fn init_usart1_pins(gpioa: &pac::GPIOA) {
    // -----------------------------------------------------------------------
    // MODE CONFIGURATION
    // -----------------------------------------------------------------------
    //
    // Set PA9 and PA10 to Alternate Function mode
    //
    gpioa.moder().modify(|_, w| {
        w.mode9().alternate(); // PA9  → AF (USART1_TX)
        w.mode10().alternate() // PA10 → AF (USART1_RX)
    });

    // -----------------------------------------------------------------------
    // ALTERNATE FUNCTION SELECTION (AF1 = USART1)
    // -----------------------------------------------------------------------
    //
    // Using indexed AFRH access:
    // - afr(1) → PA9
    // - afr(2) → PA10
    //
    gpioa.afrh().modify(|_, w| unsafe {
        w.afr(1).bits(1); // PA9  → AF1 (USART1_TX)
        w.afr(2).bits(1) // PA10 → AF1 (USART1_RX)
    });

    // -----------------------------------------------------------------------
    // OUTPUT SPEED CONFIGURATION
    // -----------------------------------------------------------------------
    //
    // High speed on TX ensures:
    // - Faster edge transitions
    // - Better signal integrity for UART waveform
    //
    // RX speed setting is irrelevant (input pin)
    //
    gpioa.ospeedr().modify(|_, w| {
        w.ospeed9().high_speed() // PA9 (TX)
    });

    // -----------------------------------------------------------------------
    // PULL-UP / PULL-DOWN CONFIGURATION
    // -----------------------------------------------------------------------
    //
    // Leave both pins floating:
    // - TX is actively driven by MCU
    // - RX is expected to be driven externally
    //
    // If required:
    // - Use pull-up for idle-high UART line
    //
    gpioa.pupdr().modify(|_, w| {
        w.pupd9().floating(); // PA9
        w.pupd10().floating() // PA10
    });
}

// ---------------------------------------------------------------------------
// RS485 DE/RE PIN INITIALIZATION (PA3)
// ---------------------------------------------------------------------------
//
// Configures:
// - PA3 as push-pull output
// - Default state = LOW (RX mode)
//
// Design:
// - HIGH → TX enable
// - LOW  → RX enable
//
pub fn init_rs485_de(gpioa: &pac::GPIOA) {
    // Set PA3 as output
    gpioa.moder().modify(|_, w| w.mode3().output());

    // Push-pull
    gpioa.otyper().modify(|_, w| w.ot3().clear_bit());

    // Low speed (sufficient)
    gpioa.ospeedr().modify(|_, w| w.ospeed3().low_speed());

    // No pull-up/down
    gpioa.pupdr().modify(|_, w| w.pupd3().floating());

    // Default: RX mode (DE LOW)
    gpioa.bsrr().write(|w| w.br3().set_bit());
}
