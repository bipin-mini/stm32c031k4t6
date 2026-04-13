#![no_std]
#![no_main]

/// ---------------------------------------------------------------------
/// APPLICATION MODULE STRUCTURE
/// ---------------------------------------------------------------------
mod bsp;
mod encoder;
mod modbus;
mod usart1;

use panic_halt as _;

use rtic::app;
use stm32c0::stm32c031 as pac;
use systick_monotonic::*;

#[app(device = pac, peripherals = true, dispatchers = [I2C, SPI, ADC])]
mod app {

    use super::*;

    // -----------------------------------------------------------------
    // MONOTONIC TIMER
    // -----------------------------------------------------------------
    #[monotonic(binds = SysTick, default = true)]
    type SysMono = Systick<1000>;

    // -----------------------------------------------------------------
    // SHARED RESOURCES
    // -----------------------------------------------------------------
    #[shared]
    struct Shared {}

    // -----------------------------------------------------------------
    // LOCAL RESOURCES
    // -----------------------------------------------------------------
    #[local]
    struct Local {
        usart1: pac::USART1,
    }

    // -----------------------------------------------------------------
    // SYSTEM INITIALIZATION
    // -----------------------------------------------------------------
    #[init]
    fn init(ctx: init::Context) -> (Shared, Local, init::Monotonics) {
        let dp = ctx.device;

        let gpioa = dp.GPIOA;
        let exti = dp.EXTI;
        let rcc = dp.RCC;
        let usart1_dev = dp.USART1;

        // Clock init
        bsp::init_clocks(&rcc);

        // GPIO init (encoder)
        bsp::init_gpioa(&gpioa);

        // USART pins
        bsp::init_usart1_pins(&gpioa);

        // EXTI init
        bsp::init_exti(&exti);

        // RS485 DE pin
        bsp::init_rs485_de(&gpioa);

        // Encoder init
        encoder::init();

        // USART init
        usart1::init(&usart1_dev, &rcc);

        // Monotonic timer
        let mono = Systick::new(ctx.core.SYST, 16_000_000);

        blink::spawn().ok();

        (
            Shared {},
            Local { usart1: usart1_dev },
            init::Monotonics(mono),
        )
    }

    // -----------------------------------------------------------------
    // ENCODER ISR
    // -----------------------------------------------------------------
    #[task(binds = EXTI0_1, priority = 2)]
    fn exti0_1(_ctx: exti0_1::Context) {
        encoder::isr();
    }

    // -----------------------------------------------------------------
    // INDEX ISR (RESERVED)
    // -----------------------------------------------------------------
    #[task(binds = EXTI2_3, priority = 2)]
    fn exti2_3(_ctx: exti2_3::Context) {}

    // -----------------------------------------------------------------
    // USART1 ISR
    // -----------------------------------------------------------------
    #[task(binds = USART1, priority = 1, local = [usart1])]
    fn usart1_irq(ctx: usart1_irq::Context) {
        usart1::isr(ctx.local.usart1);
    }

    // -----------------------------------------------------------------
    // BACKGROUND TASK
    // -----------------------------------------------------------------
    #[task]
    fn blink(_ctx: blink::Context) {
        while let Some(_b) = usart1::read() {
            // future: modbus::process_byte(_b);
        }

        blink::spawn_after(500.millis()).ok();
    }
}
