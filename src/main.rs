#![no_std]
#![no_main]

use panic_halt as _;

use rtic::app;
use stm32c0::stm32c031 as stm32c31;
use systick_monotonic::*;

#[app(device = stm32c31, peripherals = true, dispatchers = [EXTI4_15])]
mod app {
    use super::*;
    #[monotonic(binds = SysTick, default = true)]
    type SysMono = Systick<1000>;

    #[shared]
    struct Shared {
        // shared resources go here
    }

    #[local]
    struct Local {
        // local resources go here
    }

    #[init]
    fn init(ctx: init::Context) -> (Shared, Local, init::Monotonics) {
        let dp = ctx.device;

        // Ensure HSI ON
        dp.RCC.cr().modify(|_, w| w.hsion().set_bit());
        while dp.RCC.cr().read().hsirdy().bit_is_clear() {}

        // FLASH latency = 0 wait states
        dp.FLASH.acr().modify(|_, w| unsafe { w.latency().bits(0) });

        // SYSCLK = HSI
        dp.RCC.cfgr().modify(|_, w| unsafe { w.sw().bits(0) });

        // Wait until switch complete
        while dp.RCC.cfgr().read().sws().bits() != 0 {}

        let mono = Systick::new(ctx.core.SYST, 16_000_000);

        blink::spawn().ok();

        (Shared {}, Local {}, init::Monotonics(mono))
    }

    #[task]
    fn blink(_ctx: blink::Context) {
        // reschedule itself
        blink::spawn_after(500.millis()).ok();
    }
}
