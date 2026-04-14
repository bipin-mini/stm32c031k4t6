#![allow(dead_code)]

use stm32c0::stm32c031 as pac;

#[inline(always)]
pub fn init(gpio: &pac::GPIOB) {
    gpio.moder().modify(|_, w| {
        w.mode0().output();
        w.mode1().output()
    });

    gpio.otyper().modify(|_, w| {
        w.ot0().clear_bit();
        w.ot1().clear_bit()
    });

    gpio.ospeedr().modify(|_, w| {
        w.ospeed0().low_speed();
        w.ospeed1().low_speed()
    });

    gpio.pupdr().modify(|_, w| {
        w.pupd0().floating();
        w.pupd1().floating()
    });

    // SAFE STATE: both OFF
    off(gpio);
}

#[inline(always)]
pub fn low_on(gpio: &pac::GPIOB) {
    gpio.bsrr().write(|w| w.bs0().set_bit());
}

#[inline(always)]
pub fn low_off(gpio: &pac::GPIOB) {
    gpio.bsrr().write(|w| w.br0().set_bit());
}

#[inline(always)]
pub fn high_on(gpio: &pac::GPIOB) {
    gpio.bsrr().write(|w| w.bs1().set_bit());
}

#[inline(always)]
pub fn high_off(gpio: &pac::GPIOB) {
    gpio.bsrr().write(|w| w.br1().set_bit());
}

#[inline(always)]
pub fn off(gpio: &pac::GPIOB) {
    gpio.bsrr().write(|w| {
        w.br0().set_bit();
        w.br1().set_bit()
    });
}
