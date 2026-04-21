use stm32c0::stm32c031 as pac;

pub const SYSCLK_HZ: u32 = 48_000_000;

/// ---------------------------------------------------------------------------
/// 🧩 Board Support Package (BSP)
/// ---------------------------------------------------------------------------
///
/// Stateless, deterministic hardware bring-up layer.
///
/// ---------------------------------------------------------------------------
/// 🧠 DESIGN PRINCIPLES
/// ---------------------------------------------------------------------------
///
/// - Performs **one-time hardware configuration only**
/// - Fully **stateless** (no ownership, no retained state)
/// - Contains **no application logic**
/// - Independent of higher layers (encoder, modbus, UI)
///
/// ---------------------------------------------------------------------------
/// ⚙️ RESPONSIBILITIES
/// ---------------------------------------------------------------------------
///
/// - GPIO configuration (mode, type, speed, pull)
/// - EXTI configuration (edge trigger, masking)
/// - System clock configuration
/// - Minimal peripheral clock enable
///
/// ---------------------------------------------------------------------------
/// ⚠️ NON-RESPONSIBILITIES
/// ---------------------------------------------------------------------------
///
/// - No ISR logic
/// - No protocol handling
/// - No runtime decision-making
///
/// ---------------------------------------------------------------------------
/// 🔗 HARDWARE COUPLING (SYSTEM CONTRACT)
/// ---------------------------------------------------------------------------
///
/// This BSP enforces pin assumptions required by higher layers:
///
/// Encoder (must share same port for atomic sampling):
/// - PA0 → ENC_A (EXTI0)
/// - PA1 → ENC_B (EXTI1)
/// - PA2 → ENC_Z (EXTI2)
///
/// Power monitoring:
/// - PA6 → PWR_SENSE (EXTI6)
///
/// Communication:
/// - PA9  → USART1_TX
/// - PA10 → USART1_RX
/// - PA3  → RS485 DE/RE
///
/// Display (TM1638, bit-banged):
/// - PA4 → STB
/// - PA5 → CLK
/// - PA7 → DIO (bidirectional)
///
/// ---------------------------------------------------------------------------
/// 🧩 EXTI ROUTING (STM32C0)
/// ---------------------------------------------------------------------------
///
/// - EXTI0–7 default to GPIOA after reset
/// - No dynamic routing required
///
/// ---------------------------------------------------------------------------
/// GPIOA Initialization
/// ---------------------------------------------------------------------------
///
/// Configures all GPIOA pins used by the system.
///
/// ---------------------------------------------------------------------------
/// 🧠 DESIGN NOTES
/// ---------------------------------------------------------------------------
///
/// Encoder (PA0–PA2):
/// - Floating inputs (externally driven, no internal bias)
/// - Same port → enables single-cycle IDR sampling
///
/// RS485 (PA3):
/// - Push-pull output
/// - Default LOW → receive mode
///
/// TM1638 (PA4, PA5, PA7):
/// - Software-driven interface (bit-banging)
/// - STB/CLK → outputs
/// - DIO → output by default, switched dynamically in driver
///
/// Power Sense (PA6):
/// - Input with pull-up
/// - HIGH → normal operation
/// - LOW  → power failure (EXTI falling edge)
///
/// ---------------------------------------------------------------------------
/// ⚠️ ELECTRICAL ASSUMPTIONS
/// ---------------------------------------------------------------------------
///
/// - Encoder signals are clean, push-pull, CMOS-compatible
/// - TM1638 lines are short and not heavily loaded
/// - PWR_SENSE is driven by external supervisor circuit
///   providing early warning before VDD collapse
///
/// ---------------------------------------------------------------------------
pub fn init_gpioa(gpioa: &pac::GPIOA) {
    // ---------------- MODE ----------------
    gpioa.moder().modify(|_, w| {
        // Encoder
        w.mode0().input();
        w.mode1().input();
        w.mode2().input();

        // RS485 DE
        w.mode3().output();

        // TM1638
        w.mode4().output(); // STB
        w.mode5().output(); // CLK
        w.mode7().output(); // DIO (default output)

        // Power sense
        w.mode6().input()
    });

    // ---------------- OUTPUT TYPE ----------------
    gpioa.otyper().modify(|_, w| {
        w.ot3().clear_bit(); // RS485 DE
        w.ot4().clear_bit(); // STB
        w.ot5().clear_bit(); // CLK
        w.ot7().clear_bit() // DIO
    });

    // ---------------- PULL CONFIG ----------------
    gpioa.pupdr().modify(|_, w| {
        // Encoder → no internal bias
        w.pupd0().floating();
        w.pupd1().floating();
        w.pupd2().floating();

        // TM1638 → no pulls
        w.pupd4().floating();
        w.pupd5().floating();
        w.pupd7().floating();

        // RS485 DE
        w.pupd3().floating();

        // Power sense → required bias
        w.pupd6().pull_up()
    });

    // ---------------- SPEED ----------------
    gpioa.ospeedr().modify(|_, w| {
        // Encoder (low EMI, no need for speed)
        w.ospeed0().low_speed();
        w.ospeed1().low_speed();
        w.ospeed2().low_speed();

        // RS485
        w.ospeed3().low_speed();

        // TM1638 → faster edges for bit-banging reliability
        w.ospeed4().high_speed();
        w.ospeed5().high_speed();
        w.ospeed7().high_speed();

        // Power sense
        w.ospeed6().low_speed()
    });

    // ---------------- DEFAULT OUTPUT STATE ----------------
    gpioa.bsrr().write(|w| {
        // RS485 → RX mode (DE LOW)
        w.br3().set_bit();

        // TM1638 idle state:
        // STB HIGH (inactive)
        // CLK HIGH
        // DIO HIGH
        w.bs4().set_bit();
        w.bs5().set_bit();
        w.bs7().set_bit()
    });
}

/// ---------------------------------------------------------------------------
/// EXTI Initialization
/// ---------------------------------------------------------------------------
///
/// Configures interrupt lines for encoder and power monitoring.
///
/// ---------------------------------------------------------------------------
/// 🧠 DESIGN NOTES
/// ---------------------------------------------------------------------------
///
/// Encoder (EXTI0–2):
/// - Both-edge trigger → required for X4 decoding
/// - No filtering → ISR handles validation via LUT
///
/// Power Sense (EXTI6):
/// - Falling edge only (power fail detection)
/// - Isolated interrupt group (EXTI4_15)
///
/// ---------------------------------------------------------------------------
/// ⚠️ STM32C0 EDGE FLAGS
/// ---------------------------------------------------------------------------
///
/// - Rising edge → RPR1
/// - Falling edge → FPR1
/// - BOTH must be cleared in ISR
///
/// ---------------------------------------------------------------------------
pub fn init_exti(exti: &pac::EXTI) {
    // Map EXTI0–7 → GPIOA (reset default, enforced)
    exti.exticr1().write(|w| unsafe { w.bits(0x0000) });

    // Rising edges (encoder only)
    exti.rtsr1().modify(|_, w| {
        w.rt0().set_bit();
        w.rt1().set_bit();
        w.rt2().set_bit()
    });

    // Falling edges (encoder + power fail)
    exti.ftsr1().modify(|_, w| {
        w.ft0().set_bit();
        w.ft1().set_bit();
        w.ft2().set_bit();
        w.ft6().set_bit()
    });

    // Clear pending flags
    exti.rpr1().write(|w| {
        w.rpif0().set_bit();
        w.rpif1().set_bit();
        w.rpif2().set_bit();
        w.rpif6().set_bit()
    });

    exti.fpr1().write(|w| {
        w.fpif0().set_bit();
        w.fpif1().set_bit();
        w.fpif2().set_bit();
        w.fpif6().set_bit()
    });

    cortex_m::asm::dsb();
    cortex_m::asm::isb();

    // Enable interrupt lines
    exti.imr1()
        .modify(|r, w| unsafe { w.bits(r.bits() | (1 << 0) | (1 << 1) | (1 << 2) | (1 << 6)) });

    // NVIC enable
    unsafe {
        cortex_m::peripheral::NVIC::unmask(pac::Interrupt::EXTI0_1);
        cortex_m::peripheral::NVIC::unmask(pac::Interrupt::EXTI2_3);
        cortex_m::peripheral::NVIC::unmask(pac::Interrupt::EXTI4_15);
    }
}

/// ---------------------------------------------------------------------------
/// System Clock Initialization (48 MHz, HSI48)
/// ---------------------------------------------------------------------------
///
/// STM32C031 uses:
///     SYSCLK = HSI48 / HSIDIV
///
/// Reset:
///     HSIDIV = /4 → 12 MHz
///
/// Target:
///     HSIDIV = /1 → 48 MHz
///
/// ---------------------------------------------------------------------------
/// ⚠️ REQUIREMENTS
/// ---------------------------------------------------------------------------
///
/// - Flash latency MUST be set before increasing frequency
/// - No PLL available on STM32C0
///
/// ---------------------------------------------------------------------------
pub fn init_clocks(rcc: &pac::RCC) {
    let flash = unsafe { &*pac::FLASH::ptr() };

    // 1 wait state for 48 MHz
    flash.acr().modify(|_, w| unsafe { w.latency().bits(1) });

    // Set HSIDIV = /1
    rcc.cr().modify(|_, w| unsafe { w.hsidiv().bits(0) });

    // Select HSI as SYSCLK
    rcc.cfgr().modify(|_, w| unsafe { w.sw().bits(0) });

    while rcc.cfgr().read().sws().bits() != 0 {}

    // Enable required peripheral clocks
    rcc.iopenr().modify(|_, w| w.gpioaen().set_bit());
    rcc.apbenr2().modify(|_, w| w.syscfgen().set_bit());

    cortex_m::asm::dsb();
}

/// ---------------------------------------------------------------------------
/// USART1 GPIO Initialization (PA9/PA10 → AF1)
/// ---------------------------------------------------------------------------
pub fn init_usart1_pins(gpioa: &pac::GPIOA) {
    gpioa.moder().modify(|_, w| {
        w.mode9().alternate();
        w.mode10().alternate()
    });

    gpioa.afrh().modify(|_, w| unsafe {
        w.afr(1).bits(1);
        w.afr(2).bits(1)
    });

    gpioa.ospeedr().modify(|_, w| w.ospeed9().high_speed());

    gpioa.pupdr().modify(|_, w| {
        w.pupd9().floating();
        w.pupd10().floating()
    });

    unsafe {
        cortex_m::peripheral::NVIC::unmask(pac::Interrupt::USART1);
    }
}

/// ---------------------------------------------------------------------------
/// RS485 DE Pin Initialization (PA3)
/// ---------------------------------------------------------------------------
///
/// - Push-pull output
/// - LOW → receive
/// - HIGH → transmit
///
pub fn init_rs485_de(gpioa: &pac::GPIOA) {
    gpioa.moder().modify(|_, w| w.mode3().output());
    gpioa.otyper().modify(|_, w| w.ot3().clear_bit());
    gpioa.ospeedr().modify(|_, w| w.ospeed3().low_speed());
    gpioa.pupdr().modify(|_, w| w.pupd3().floating());

    // Default to RX mode
    gpioa.bsrr().write(|w| w.br3().set_bit());
}


