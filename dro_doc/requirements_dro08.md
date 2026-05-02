# 📄 Firmware Requirement Specification

## Digital Readout for Quadrature Encoder

---

## 1. System Overview

This firmware implements a **Digital Readout (DRO)** system for an incremental quadrature encoder. The system performs:

* Real-time pulse counting using software-based quadrature decoding
* Conversion to engineering units using scaling factor
* Local display via 7-segment interface
* Communication via Modbus RTU (RS485)
* Persistent storage of configuration and encoder count
* Dual relay-based limit monitoring (LOW/HIGH thresholds)

---

## 2. Hardware Configuration

| Component       | Specification                                             |
| --------------- | --------------------------------------------------------- |
| Microcontroller | STM32C031K4T6 @ 48 MHz                                    |
| Power Supply    | 3.3V DC                                                   |
| Encoder Type    | Incremental Quadrature Encoder with Index (Z)             |
| Display         | 6-digit 7-segment with decimal point + dedicated sign LED |
| Display Driver  | TM1638 (CLK, DIO, STB + key scan)                         |
| User Input      | Push buttons via TM1638 key scan                          |
| Communication   | UART (Modbus RTU over RS485)                              |
| Outputs         | 2 Relay Outputs (LOW / HIGH)                              |

---

## 3. Encoder Interface

### 3.1 Quadrature Signals (A, B)

* Interrupt-driven GPIO (both edges)
* 4-bit LUT decoding (branchless)
* Maximum rate: X4 → 100,000 edges/sec

### 🔹 Quadrature Decode Algorithm (Mandatory)

* Previous and current state form 4-bit index
* LUT output: {-1, 0, +1}
* Result accumulated into pulse counter

#### State Handling (MANDATORY)

* ISR shall read both A and B from GPIO on every interrupt
* ISR must NOT rely on which EXTI line triggered
* Previous state must be stored and updated only after LUT evaluation

#### Constraints

* Branchless implementation
* Cycle-invariant execution time
* Executed entirely inside ISR
* LUT in RAM or zero-wait memory

### 🔹 EXTI Configuration (Mandatory)

* Interrupt on both edges (A & B)
* Separate EXTI lines (no sharing)
* Same priority level (no preemption among A/B)

### 🔹 Atomic Sampling Requirement

* Encoder A and B shall be on same GPIO port
* Must be sampled via single IDR read

### 🔹 Latency Constraint

* Worst-case latency < minimum encoder edge interval
* Minimum valid pulse width ≥ 800 ns
* Minimum pulse width ≥ 5 × ISR latency

### 🔹 Interrupt Load Requirement

* System must sustain ≥ 100,000 interrupts/sec
* All pending EXTI events must be serviced sequentially (no intentional drop)

### 🔹 Input Signal Integrity

* Inputs must be CMOS/TTL compatible
* Hardware filtering or software glitch rejection required
* PCB routing must minimize EMI and crosstalk

### 3.2 Index Pulse (Z)

* Rising-edge interrupt

#### Behavior

* Shall NOT modify pulse counter
* Shall latch position and set flag

---

## 4. Pulse Counter

* 32-bit signed (`int32_t`)
* Wraparound behavior

### Requirements

* Modified only inside encoder ISRs
* No higher-priority ISR shall access or modify counter

---

## 5. Scaling

* Fixed-point (5 decimals)

### Range

* Min: 0.00001
* Max: 999999.00000

### Conversion

```
Engineering Value = Pulse Count × Scaling Factor
```

* int64 intermediate
* Truncate toward zero

### Overflow Handling

* Clamp to ±999999

---

## 6. Display System

* TM1638 based
* No blocking operations in ISR

### Display Priority

1. Error codes
2. Overflow/Underflow
3. Normal value

---

## 7. User Interface

| Key  | Function      |
| ---- | ------------- |
| KEY1 | Increment     |
| KEY2 | Decrement     |
| KEY3 | Confirm       |
| KEY4 | Cancel        |
| KEY5 | Configuration |
| KEY6 | Counter Reset |

---

## 8. Modbus RTU (RS485)

* 9600, 8N1
* Modbus slave address stored in eeprom
* Address shall be editable via push buttons (UI configuration mode)

### Address Configuration Requirements

* Valid range: 1–247
* Default address must be defined
* Changes must be persisted to flash with CRC validation
* New address becomes active only after confirmation

### RS485 Control

* DE HIGH → TX

* DE LOW → RX

### Timing Requirements

* DE asserted before first byte
* DE held until TC flag set
* DE released within ≤ 1 bit time
* RX enabled immediately after DE low

### Modbus Timing

* Inter-frame gap ≥ 3.5 char times
* Frame timeout based on silence ≥ 3.5 char times

---

## 9. Relay Control

* Executed outside ISR

### Fault Handling

* Relay shutdown ≤ 10 ms after fault

---

## 10. Non-Volatile Storage

* Flash-based

### Data Integrity

* CRC required
* Invalid data → defaults

---

## 11. Encoder Persistence

* Interrupts disabled
* No pending ISR allowed

---

## 12. Power Management

### Power Sense (MANDATORY)

* Dedicated GPIO with EXTI interrupt
* Falling edge indicates power failure
* Must use non-shared EXTI line
* Signal must transition before VDD falls below safe operating level

### Safe Shutdown

* Triggered by power-fail interrupt
* Relays OFF
* Encoder count saved
* Communication stopped

### Hold-up Requirement

* Hold-up time ≥ 2× worst-case flash write time

---

## 13. Timing Requirements

| Function          | Requirement |
| ----------------- | ----------- |
| Encoder ISR       | ≤ 80 cycles |
| ISR body target   | ≤ 50 cycles |
| Encoder rate      | ≤ 100k/sec  |
| Interrupt latency | ≤ 2 µs      |
| Processing        | ≥ 1 kHz     |
| Relay update      | ≥ 100 Hz    |
| Display update    | ≥ 10 Hz     |

### System Guarantee

* ISR execution time must be input-independent

---

## 14. Firmware Architecture

### Priority Order

1. Power-fail
2. Encoder A/B
3. Index
4. UART
5. SysTick
6. Others

### ISR Rules

* Encoder ISR: counting only
* Power-fail ISR: flag only (no flash or blocking operations)
* No scaling/UI/Modbus inside ISR

### Main Loop

* Scaling
* Display
* Modbus
* Relay logic
* Power-fail handling sequence

---

## 15. Reliability

### Watchdog

* Timeout ≤ 100 ms
* Refreshed only in main loop
* System must guarantee ISR load does not block refresh

---

## 16. System Modes

* Normal
* Configuration
* Fault
* Power-fail

---

## 17. Testing

* Cycle-accurate ISR timing
* Interrupt stress test at max rate
* EMI/noise validation
* Modbus timing validation

---

# 📍 18. Pin Mapping (Optimized)

## Encoder

| Signal | Pin |
| ------ | --- |
| ENC_A  | PA0 |
| ENC_B  | PA1 |
| ENC_Z  | PA2 |

## TM1638

| Signal | Pin |
| ------ | --- |
| STB    | PA4 |
| CLK    | PA5 |
| DIO    | PA7 |

## UART RS485

| Signal | Pin  |
| ------ | ---- |
| TX     | PA9  |
| RX     | PA10 |
| DE/RE  | PA3  |

## Power Sense

| Signal    | Pin |
| --------- | --- |
| PWR_SENSE | PA6 |

## Relay

| Function | Pin |
| -------- | --- |
| LOW      | PB0 |
| HIGH     | PB1 |

## Debug / Programming (SWD)

| Signal | Pin  |
| ------ | ---- |
| SWDIO  | PA13 |
| SWCLK  | PA14 |
| NRST   | NRST |

---

## Constraints

* Same GPIO port for encoder
* No EXTI sharing (including power sense)
* No HAL inside ISR
* SWD pins shall not be used for application I/O

---

## Result

* Deterministic
* High-speed capable
* Industrial-ready

---

**END OF DOCUMENT**
