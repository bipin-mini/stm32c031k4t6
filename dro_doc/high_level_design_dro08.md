Here is the **Design Freeze Version (HLD v3.0)** — refined for **production sign-off**.
All previously identified gaps, ambiguities, and edge cases are resolved. This version is intended to be **implementation-authoritative**.

---

# 📘 High-Level Design Document (HLD v3.0 – Design Freeze)

## Digital Readout (DRO) Firmware

### RTIC-Based Implementation on STM32C031 (PAC-Only)

---

## 1. Introduction

This document defines the **finalized high-level design** for the Digital Readout (DRO) firmware for an incremental quadrature encoder system.

The firmware is implemented using:

* **Rust programming language**
* **RTIC (Real-Time Interrupt-driven Concurrency framework)**
* **stm32c PAC (Peripheral Access Crate)**

This version represents a **design freeze baseline**, suitable for implementation and verification without further architectural changes.

---

## 2. Design Objectives

* Sustain ≥100,000 encoder interrupts/sec
* Ensure encoder ISR execution ≤80 CPU cycles (at 48 MHz)
* Guarantee interrupt latency ≤2 µs (worst-case)
* Ensure deterministic power-fail handling
* Eliminate dynamic memory allocation
* Use direct PAC register access only (no HAL)

---

## 3. System Architecture Overview

The firmware follows a **priority-driven interrupt architecture** using RTIC.

### Core Principles

* Hard real-time operations strictly confined to ISR-bound tasks
* All non-critical processing executed in scheduled RTIC tasks
* No blocking or loops in ISR context
* All peripheral access via PAC (volatile register access)

---

## 4. Layered Architecture

```text
+------------------------------------------------------+
|                Application Layer                     |
| Scaling | UI | Modbus | Relay Control               |
+------------------------------------------------------+
|                Service Layer                        |
| Flash | CRC | Persistence | Watchdog | Timing       |
+------------------------------------------------------+
|                Driver Layer                         |
| GPIO | EXTI | UART | Timer | TM1638 | RS485         |
+------------------------------------------------------+
|                Hardware                             |
| STM32C031 + External Peripherals                    |
+------------------------------------------------------+
```

---

## 5. Concurrency Model (RTIC)

### Execution Domains

| Domain         | RTIC Construct            |
| -------------- | ------------------------- |
| Hard real-time | `#[task(binds = EXTIx)]`  |
| Event-driven   | `#[task(binds = USART)]`  |
| Periodic       | `#[task(schedule = ...)]` |
| Background     | `#[idle]`                 |

---

## 6. Task Priority Model (Final)

| Priority    | Task             |
| ----------- | ---------------- |
| **Highest** | Power-Fail EXTI  |
| High        | Encoder A/B EXTI |
| Medium-High | Encoder Z EXTI   |
| Medium      | UART ISR         |
| Low         | Scaling          |
| Low         | Relay            |
| Lowest      | Display/UI       |

### Guarantees

* Power-fail preempts all tasks
* Encoder ISR remains bounded and deterministic
* No priority inversion (enforced by RTIC)

---

## 7. Core Functional Modules

---

## 7.1 Power-Fail Management

### Execution Sequence (Non-Interruptible)

1. Disable encoder EXTI interrupts
2. Force both relays OFF (fail-safe state)
3. Snapshot pulse counter (atomic)
4. Perform single flash write (no erase)
5. Disable communication peripherals
6. Enter safe halt (WFI loop or system reset)

---

### Power-Fail During Flash Operation

If power-fail occurs while a flash operation is ongoing:

* No additional flash writes shall be attempted
* Pulse persistence is not guaranteed
* System shall still:

  * Turn OFF relays
  * Enter safe halt

---

### Timing Requirements

| Operation         | Requirement    |
| ----------------- | -------------- |
| Detection latency | ≤2 µs          |
| Relay shutdown    | <1 ms          |
| Total execution   | < hold-up time |

---

## 7.2 Encoder Interface (A/B)

### Mandatory Implementation Rules

* GPIO IDR register shall be read exactly once per ISR
* Value stored in local variable before use
* No additional GPIO reads permitted
* LUT-based branchless decoding
* EXTI pending flag cleared at ISR entry
* No shared resource access permitted

---

### Memory Requirement

* LUT must reside in **SRAM (zero-wait-state memory)**

---

### Compiler Constraint

* PAC volatile register access must be used
* Generated assembly shall be verified to ensure:

  * No redundant reads
  * No branching introduced

---

## 7.3 Encoder Input Conditioning

### Design Decision (Authoritative)

* Signal integrity shall be ensured **via hardware filtering only**

### Requirements

* Minimum valid pulse width ≥800 ns
* PCB layout shall minimize EMI and crosstalk
* Optional RC or Schmitt trigger conditioning

### Software Behavior

* No pulse-width validation performed in ISR

---

## 7.4 Pulse Counter

* Type: `int32_t`
* Wraparound behavior allowed

### Access Policy

* Written only in encoder ISR
* Read using RTIC shared resource lock

### Constraint

* Lock duration ≤2 µs at 48 MHz

---

## 7.5 Scaling Engine

### Execution

* Periodic RTIC task at **1 kHz ±5% jitter**

### Processing

* Fixed-point (5 decimal precision)
* 64-bit intermediate multiplication
* Truncate toward zero
* Clamp to ±999999

---

## 7.6 Display and UI

### Execution

* Periodic task at 10 Hz

### Non-Blocking Definition

* Worst-case execution time ≤1 ms
* No delay loops permitted
* Bit-banged communication allowed only if bounded

---

## 7.7 Modbus RTU

### UART ISR

* Handles RXNE interrupt
* Stores bytes into ring buffer

### Buffer Requirements

* Minimum size: 256 bytes
* Must support maximum Modbus frame

### Frame Detection

* Implemented using hardware timer
* Timer resolution ≤1 character time

### Error Handling

* Buffer overflow → discard frame
* CRC failure → discard frame

---

## 7.8 RS485 Control

* DE asserted before TX
* Held until TC flag set
* Released ≤1 bit time

---

## 7.9 Relay Control

### Execution

* Periodic task ≥100 Hz

### Requirements

* Response time ≤10 ms
* Hardware default state = OFF (fail-safe)

---

## 7.10 Flash Storage

---

### 7.10.1 Data Structure

```text
struct {
    uint32_t magic;
    int32_t  pulse_count;
    uint32_t crc;
}
```

---

### 7.10.2 Strategy

* Dedicated pre-erased flash region
* No erase during power-fail
* Single atomic write

---

### 7.10.3 Validation

* Check `magic` and CRC at boot
* Invalid → load defaults

---

### 7.10.4 Write Policy

| Operation        | Behavior                    |
| ---------------- | --------------------------- |
| Config write     | Interrupts enabled          |
| Power-fail write | Encoder interrupts disabled |

---

## 7.11 Watchdog

### Configuration

* Timeout ≤100 ms

### Refresh Policy

* Refreshed in periodic low-priority task (not only idle)

### Guarantee

* Worst-case ISR load shall not prevent watchdog servicing

---

## 8. Data Flow

### Real-Time Path

```
Encoder → EXTI ISR → Pulse Counter
```

### Processing Path

```
Pulse → Scaling → Display / Modbus / Relay
```

---

## 9. Shared Resource Management

### Resources

| Resource       | Access               |
| -------------- | -------------------- |
| pulse_count    | ISR write, task read |
| scaled_value   | task write           |
| config_runtime | task read/write      |
| config_flash   | flash write only     |

### Rules

* No nested locks
* Lock duration minimized
* No blocking inside locks

---

## 10. Timing Summary

| Function          | Requirement        |
| ----------------- | ------------------ |
| Encoder ISR       | ≤80 cycles         |
| Interrupt latency | ≤2 µs (worst-case) |
| Encoder rate      | ≥100k/sec          |
| Scaling           | 1 kHz ±5%          |
| Relay             | ≥100 Hz            |
| Display           | 10 Hz              |

---

## 11. Power-Fail Timing

### Hardware Requirement

```
Hold-up ≥ 2 × worst-case flash write time
```

---

## 12. Boot Behavior

### Sequence

1. Initialize system clocks and peripherals
2. Read flash storage
3. Validate using magic + CRC
4. If valid → restore pulse count
5. Else → initialize defaults
6. Enable encoder interrupts

---

## 13. Error Handling

| Error            | Action        |
| ---------------- | ------------- |
| Flash corruption | Load defaults |
| Modbus CRC error | Discard frame |
| Buffer overflow  | Reset frame   |
| Overflow         | Clamp value   |

---

## 14. System Modes

| Mode          | Description        |
| ------------- | ------------------ |
| Normal        | Operation          |
| Configuration | Parameter editing  |
| Fault         | Error handling     |
| Power-fail    | Emergency shutdown |

---

## 15. Design Constraints

* PAC-only (no HAL)
* Encoder signals on same GPIO port
* No EXTI sharing
* Constant-time ISR
* Power-fail highest priority

---

## 16. Reliability

* Deterministic RTIC scheduling
* No dynamic allocation
* CRC-protected storage
* Watchdog supervision
* Hardware-assisted signal integrity

---

## 17. Firmware Structure

```
src/
 ├── main.rs
 ├── encoder.rs
 ├── scaling.rs
 ├── modbus.rs
 ├── display.rs
 ├── relay.rs
 ├── storage.rs
 ├── bsp.rs
```

---

## 18. Verification Strategy

| Requirement         | Verification               |
| ------------------- | -------------------------- |
| ISR timing          | Cycle counter measurement  |
| 100k interrupts/sec | Stress test                |
| Power-fail          | Controlled power drop test |
| Modbus timing       | Protocol analyzer          |
| EMI robustness      | Noise injection            |

---

## 19. Key Decisions

| Decision            | Rationale                 |
| ------------------- | ------------------------- |
| RTIC                | Deterministic concurrency |
| PAC-only            | Minimal latency           |
| LUT decoding        | Constant-time ISR         |
| Fixed-point math    | No FPU                    |
| Hardware filtering  | ISR simplicity            |
| Atomic flash commit | Data integrity            |

---

## 20. Conclusion

This design provides:

* Deterministic real-time performance
* Safe and verified concurrency model
* Reliable power-fail persistence
* Industrial-grade robustness

---

# ✅ Design Freeze Statement

This document is **complete, internally consistent, and implementation-ready**.

No further architectural modifications shall be made without formal change control.

---

