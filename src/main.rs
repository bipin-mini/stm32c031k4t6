#![no_std]
#![no_main]

use panic_halt as _;
use stm32c0::stm32c031 as pac;

mod bsp;
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
use systick_monotonic::*;

#[derive(Copy, Clone)]
pub struct PowerSnapshot {
    encoder: u64,
    valid: bool,
}

#[app(device = pac, peripherals = true, dispatchers = [I2C, SPI, ADC])]
mod app {

    use super::*;
    use crate::drivers::uart::Uart;
    use crate::modbus::Modbus;
    use crate::storage::eeprom::Eeprom;

    #[monotonic(binds = SysTick, default = true)]
    type SysMono = Systick<1000>;

    #[shared]
    struct Shared {
        snapshot: PowerSnapshot,
    }

    #[local]
    struct Local {
        uart: Uart,
        modbus: Modbus,
        eeprom: Eeprom,
        gpiob: pac::GPIOB,
    }

    #[init]
    fn init(ctx: init::Context) -> (Shared, Local, init::Monotonics) {
        let dp = ctx.device;

        // BSP
        bsp::init_clocks(&dp.RCC);
        bsp::init_gpioa(&dp.GPIOA);
        bsp::init_usart1_pins(&dp.GPIOA);
        bsp::init_rs485_de(&dp.GPIOA);

        // ✅ FIX: use GPIOB for I2C
        bsp::init_i2c1_pins(&dp.GPIOB);

        bsp::init_exti(&dp.EXTI);

        // Drivers
        drivers::encoder::init();
        drivers::relay::init(&dp.GPIOB);
        drivers::tm1638::init();

        let uart = Uart::new(dp.USART1, &dp.RCC);
        let modbus = Modbus::new();
        let eeprom = Eeprom::new(dp.I2C1, &dp.RCC);

        let mono = Systick::new(ctx.core.SYST, SYSCLK_HZ);

        (
            Shared {
                snapshot: PowerSnapshot {
                    encoder: 0,
                    valid: false,
                },
            },
            Local {
                uart,
                modbus,
                eeprom,
                gpiob: dp.GPIOB,
            },
            init::Monotonics(mono),
        )
    }

    #[task(binds = EXTI0_1, priority = 2)]
    fn exti0_1(_ctx: exti0_1::Context) {
        drivers::encoder::isr();
    }

    #[task(binds = EXTI4_15, priority = 3, shared = [snapshot])]
    fn power_fail_irq(mut ctx: power_fail_irq::Context) {
        cortex_m::interrupt::disable();

        let encoder = drivers::encoder::get_count() as u64;

        ctx.shared.snapshot.lock(|snap| {
            snap.encoder = encoder;
            snap.valid = true;
        });

        let exti = unsafe { &*pac::EXTI::ptr() };
        const MASK: u32 = 1 << 6;

        exti.rpr1().write(|w| unsafe { w.bits(MASK) });
        exti.fpr1().write(|w| unsafe { w.bits(MASK) });
    }

    #[task(binds = USART1, priority = 1, local = [uart, modbus])]
    fn usart1_irq(ctx: usart1_irq::Context) {
        let uart = ctx.local.uart;
        let usart = &uart.usart;

        let isr = usart.isr().read();

        // ---------------- RX BYTE ----------------
        if isr.rxfne().bit_is_set() {
            let b = usart.rdr().read().bits() as u8;
            ctx.local.modbus.push_byte(b);
        }

        // ---------------- FRAME COMPLETE ----------------
        if isr.rtof().bit_is_set() {
            usart.icr().write(|w| w.rtocf().clear());

            ctx.local.modbus.process(uart);
        }

        // ---------------- TX COMPLETE ----------------
        if isr.tc().bit_is_set() {
            usart.icr().write(|w| w.tccf().clear());
        }
    }
    #[idle(shared = [snapshot], local = [eeprom, gpiob])]
    fn idle(mut ctx: idle::Context) -> ! {
        loop {
            let mut do_commit = None;

            ctx.shared.snapshot.lock(|snap| {
                if snap.valid {
                    do_commit = Some(snap.encoder);
                    snap.valid = false;
                }
            });

            if let Some(encoder) = do_commit {
                let bytes = encoder.to_le_bytes();
                ctx.local.eeprom.write(0x01, &bytes);

                drivers::relay::off(ctx.local.gpiob);

                loop {
                    cortex_m::asm::nop();
                }
            }

            cortex_m::asm::wfi();
        }
    }
}
