Below is your **fully updated HLD v3.0 (Design Freeze – Clean Driver Consolidation Edition)** with the requested structural change fully integrated and consistently reflected across the document.

No architectural changes were introduced beyond the **drivers folder consolidation + tm1638/display update consistency**.

---

# 📘 High-Level Design Document (HLD v3.0 – Design Freeze)

## Digital Readout (DRO) Firmware

### RTIC-Based Implementation on STM32C031 (PAC-Only)

---

## 1. Introduction

This document defines the finalized high-level design for the Digital Readout (DRO) firmware for an incremental quadrature encoder system.

The firmware is implemented using:

* Rust programming language
* RTIC (Real-Time Interrupt-driven Concurrency framework)
* stm32c PAC (Peripheral Access Crate only)

This version is the **design freeze baseline** for implementation and verification.

---

## 2. Design Objectives

* Sustain ≥100,000 encoder interrupts/sec
* Encoder ISR execution ≤80 CPU cycles @ 48 MHz
* Interrupt latency ≤2 µs worst-case
* Deterministic power-fail handling
* No dynamic allocation
* PAC-only hardware access (no HAL abstraction)

---

## 3. System Architecture Overview

```text
+------------------------------------------------------+
|                Application Layer                     |
| Scaling | UI | Modbus | Relay Control               |
+------------------------------------------------------+
|                Service Layer                        |
| Flash | CRC | Persistence | Watchdog | Timing       |
+------------------------------------------------------+
|                Driver Layer                         |
| GPIO | EXTI | UART | TM1638 | RS485 | Encoder       |
+------------------------------------------------------+
|                Hardware                             |
| STM32C031 + External Peripherals                    |
+------------------------------------------------------+
```

---

## 4. Firmware Source Structure (FINAL)

```text
src/
 ├── main.rs
 │
 ├── bsp.rs
 │
 ├── drivers/
 │    ├── encoder.rs
 │    ├── flash.rs
 │    ├── tm1638.rs
 │    ├── relay.rs
 │    ├── usart.rs
 │
 ├── scaling.rs
 │
 ├── modbus.rs
```

---

## 5. Concurrency Model (RTIC)

| Domain         | RTIC Construct            |
| -------------- | ------------------------- |
| Hard real-time | `#[task(binds = EXTIx)]`  |
| Event-driven   | `#[task(binds = USART)]`  |
| Periodic       | `#[task(schedule = ...)]` |
| Background     | `#[idle]`                 |

---

## 6. Task Priority Model

| Priority    | Task            |
| ----------- | --------------- |
| Highest     | Power-Fail EXTI |
| High        | Encoder EXTI    |
| Medium-High | Index EXTI      |
| Medium      | UART ISR        |
| Low         | Scaling         |
| Low         | Relay control   |
| Lowest      | Display/UI      |

---

## 7. Core Functional Modules

---

## 7.1 Power-Fail Management

* Immediate EXTI trigger on PA6
* Relay OFF first (fail-safe)
* Snapshot pulse counter
* Flash write (single atomic operation)
* Communication shutdown
* Enter safe halt

### Constraint

* No flash erase during emergency write

---

## 7.2 Encoder Interface (A/B)

* EXTI on both edges
* Single GPIO IDR read per ISR
* LUT-based branchless decoding
* Zero branching ISR requirement
* Strict constant-time execution

---

## 7.3 Pulse Counter

* Type: `int32_t`
* Written only in ISR
* Read via RTIC shared resource
* Wraparound allowed

---

## 7.4 Scaling Engine

* 1 kHz RTIC task
* Fixed-point arithmetic (5 decimals)
* 64-bit intermediate math
* Clamp to ±999999

---

## 7.5 Display System (TM1638)

Driver location:

```text
drivers/tm1638.rs
```

* 7-segment multiplexed output
* Bit-banged interface
* Non-blocking updates
* Max update rate: 10 Hz

---

## 7.6 Modbus RTU System

* UART RX interrupt → ring buffer
* Frame parsing in system layer
* CRC validation required
* 3.5 character silence detection

---

## 7.7 RS485 Control

* DE asserted before TX
* Held until TC flag
* Released within 1 bit time

---

## 7.8 Relay Control

* Periodic 100 Hz task
* Fail-safe default OFF state
* Direct GPIO driver only

---

## 7.9 Flash Storage

Driver location:

```text
drivers/flash.rs
```

### Data Format

```text
{
    magic,
    pulse_count,
    crc
}
```

### Behavior

* Page-based erase
* Single atomic write
* CRC validation at boot
* Invalid → defaults loaded

---

## 7.10 Watchdog

* Timeout ≤100 ms
* Refreshed in periodic task
* Must remain independent of ISR load

---

## 8. Data Flow

### Real-time path

```text
Encoder → ISR → Pulse Counter
```

### Processing path

```text
Pulse Counter → Scaling → Display / Modbus / Relay
```

---

## 9. Shared Resource Policy

| Resource     | Access Pattern        |
| ------------ | --------------------- |
| pulse_count  | ISR write / task read |
| scaled_value | task write            |
| config       | system layer only     |

---

## 10. Timing Requirements

| Function          | Requirement |
| ----------------- | ----------- |
| Encoder ISR       | ≤80 cycles  |
| Interrupt latency | ≤2 µs       |
| Encoder rate      | ≥100k/sec   |
| Scaling           | 1 kHz       |
| Relay update      | ≥100 Hz     |
| Display update    | 10 Hz       |

---

## 11. Power-Fail Timing

* Hold-up time ≥ 2× flash write time
* Relay shutdown < 1 ms
* Detection latency ≤ 2 µs

---

## 12. Boot Sequence

1. Clock init
2. Peripheral init
3. Flash read
4. CRC + magic validation
5. Restore state
6. Enable encoder ISR

---

## 13. Error Handling

| Condition        | Action        |
| ---------------- | ------------- |
| Flash corruption | Load defaults |
| Modbus CRC fail  | Discard frame |
| Buffer overflow  | Reset frame   |
| Overflow         | Clamp output  |

---

## 14. System Modes

* Normal
* Configuration
* Fault
* Power-fail

---

## 15. Design Constraints

* PAC-only (no HAL)
* Encoder signals same GPIO port
* No EXTI sharing
* ISR constant-time requirement
* Power-fail highest priority

---

## 16. Reliability Features

* Deterministic RTIC scheduling
* CRC-protected flash storage
* Hardware-based signal integrity
* Watchdog supervision

---

## 17. Firmware Structure (FINAL CONFIRMED)

```text
src/
 ├── main.rs
 │
 ├── bsp.rs
 │
 ├── drivers/
 │    ├── encoder.rs
 │    ├── flash.rs
 │    ├── tm1638.rs
 │    ├── relay.rs
 │    ├── usart.rs
 │
 ├── scaling.rs
 │
 ├── modbus.rs
```

---

## 18. Verification Strategy

| Requirement       | Method                 |
| ----------------- | ---------------------- |
| ISR timing        | Cycle counter analysis |
| 100k interrupts/s | Stress test            |
| Power-fail        | Controlled brownout    |
| Modbus timing     | Protocol analyzer      |
| EMI robustness    | Noise injection        |

---

## 19. Key Design Decisions

* RTIC → deterministic concurrency
* PAC-only → minimal latency
* LUT decoding → constant-time ISR
* Fixed-point math → no FPU dependency
* Hardware filtering → ISR simplicity
* Atomic flash writes → data integrity

---

## 20. Conclusion

This design provides:

* Deterministic real-time behavior
* Strict ISR execution guarantees
* Robust flash persistence model
* Clean driver/system separation
* Industrial-grade embedded architecture

---

# ✅ Design Freeze Statement

This document is **implementation-authoritative and structurally frozen**.
Only controlled change requests may modify structure or timing guarantees.
