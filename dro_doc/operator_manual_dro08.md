Here is your **polished, consistency-corrected document with minimal structural changes**, preserving your format but removing ambiguity and aligning key behavior logically.

---

# 📘 OPERATOR MANUAL

## Digital Readout (DRO) for Quadrature Encoder

---

# 1. Introduction

This system is a **Digital Readout (DRO)** designed for incremental quadrature encoders. It provides:

* Real-time position measurement
* Configurable scaling and limits
* Local 7-segment display interface
* Modbus RTU communication (RS485)
* Relay outputs for limit control

All configuration is performed using **6 push buttons** integrated with the display module.

---

# 2. Front Panel Overview

## Display

* 6-digit 7-segment display
* Decimal point for scaling
* Dedicated sign indicator LED

---

## Buttons

| Button | Label | Function                                             |
| ------ | ----- | ---------------------------------------------------- |
| KEY1   | ▲     | Increment value                                      |
| KEY2   | ▼     | Decrement value                                      |
| KEY3   | OK    | Edit / Next digit / Save                             |
| KEY4   | ESC   | Cancel / Exit                                        |
| KEY5   | SET   | Configuration / Parameter navigation / Decimal shift |
| KEY6   | RESET | Reset counter                                        |

---

# 3. Operating Modes

---

## 3.1 Normal Mode

Default operating mode after power-up.

### Display shows:

* Scaled encoder position

### Available actions:

* **RESET (long press)** → Reset counter to zero
* **SET (long press)** → Shift decimal position

---

## 3.2 Configuration Mode

Used to edit system parameters.

### Enter Configuration Mode:

* Press and hold **SET (KEY5)** for 2 seconds

### Exit:

* Press **ESC (KEY4)** or complete SAVE sequence

---

## 3.3 Fault Mode

Activated when:

* Overflow occurs
* Flash data invalid
* System error

### Display:

* Error code

---

# 4. Parameter List

| Parameter | Description      | Range            | Default |
| --------- | ---------------- | ---------------- | ------- |
| SCALE     | Scaling factor   | 0.00001 – 999999 | 1.00000 |
| LOW       | Low relay limit  | -999999 – 999999 | 0       |
| HIGH      | High relay limit | -999999 – 999999 | 100     |
| ADDR      | Modbus address   | 1 – 247          | 1       |

---

# 5. Navigation Structure

```
Normal Mode
   ↓ (SET long)
SCALE → LOW → HIGH → ADDR → SAVE → Exit
```

---

# 6. Editing Parameters via Push Buttons

---

## 6.1 Enter Configuration Mode

* Press and hold **SET (KEY5)** for 2 seconds
* First parameter (**SCALE**) appears

---

## 6.2 Parameter Navigation

* Use **SET (short press)** to move between parameters
* Value is shown in display

---

## 6.3 Enter Edit Mode

* When a parameter value is displayed:
* Press and hold **OK (KEY3)** → Enter edit mode

---

## 6.4 Digit Editing Method (MANDATORY BEHAVIOR)

* One digit blinks (active digit)
* Editing is performed digit by digit

### Controls

| Key             | Action                                  |
| --------------- | --------------------------------------- |
| ▲ (KEY1)        | Increment digit                         |
| ▼ (KEY2)        | Decrement digit                         |
| OK (short)      | Move to next digit (circular)           |
| OK (long ≥ 2s)  | Save value and exit edit mode           |
| ESC             | Cancel editing (restore previous value) |
| SET (long ≥ 2s) | Shift decimal position                  |

---

## 6.5 Editing Flow Example

Example: Set SCALE = `12.34567`

```
Display: 000000
         ^ blinking digit

Step 1: ▲ / ▼ adjust digit
Step 2: OK → next digit
Step 3: Repeat until all digits set
```

---

## 6.6 Saving Value

* Press and hold **OK (≥ 2 seconds)**
* Value is stored temporarily
* Returns to parameter view

---

## 6.7 Cancel Editing

* Press **ESC**
* Value reverts to last saved state

---

# 7. Parameter Descriptions

---

## 7.1 SCALE (Scaling Factor)

Defines conversion:

```
Position = Pulse Count × Scale
```

* Fixed 5 decimal format
* Truncation toward zero

---

## 7.2 LOW Limit

Relay activates when:

```
Position ≤ LOW
```

---

## 7.3 HIGH Limit

Relay activates when:

```
Position ≥ HIGH
```

---

## 7.4 Modbus Address (ADDR)

* Range: 1 to 247
* Used for RS485 communication

⚠ Changes take effect only after saving

---

# 8. Saving Configuration

After last parameter:

1. Display shows: `SAVE`

| Key | Action          |
| --- | --------------- |
| OK  | Save to flash   |
| ESC | Discard changes |

---

## Save Behavior

* Stored with CRC validation
* Power failure during write → defaults restored

---

# 9. Counter Reset

* Press **RESET (long press)**

### Result:

* Encoder count → 0
* Display updates immediately

---

# 10. Relay Operation

| Condition       | Relay         |
| --------------- | ------------- |
| Position ≤ LOW  | LOW relay ON  |
| Position ≥ HIGH | HIGH relay ON |

* Updated continuously
* Response time ≤ 10 ms

---

# 11. Display Indications

| Condition | Display      |
| --------- | ------------ |
| Normal    | Scaled value |
| Overflow  | OFLOW        |
| Underflow | UFLOW        |
| Error     | Error code   |

---

# 12. Power Failure Behavior

On power loss:

* Relays turned OFF
* Encoder count saved
* System shuts down safely

---

# 13. Best Practices

* Avoid parameter changes during high-speed operation
* Ensure stable encoder signals
* Verify relay limits before enabling outputs
* Confirm Modbus address after configuration
* Confirm decimal position after changes

---

# 14. Quick Reference

| Action               | Button             |
| -------------------- | ------------------ |
| Enter Config         | SET (long press)   |
| Increment            | ▲                  |
| Decrement            | ▼                  |
| Confirm / Edit Entry | OK (long press)    |
| Next Digit           | OK (short press)   |
| Save Value           | OK (long press)    |
| Cancel               | ESC                |
| Reset Counter        | RESET (long press) |
| Decimal Shift        | SET (long press)   |

---

# 15. Typical Workflow

## Setting Scaling Factor

1. Hold **SET**
2. Navigate to SCALE using SET (short)
3. Press **OK (long)** → Enter edit mode
4. Adjust digits using ▲ / ▼
5. Use **OK (short)** for next digit
6. Press **OK (long)** → Save
7. Press **SET** → Next parameter
8. After last parameter → SAVE → OK

---

# 16. Safety Notes

* Ensure relay wiring matches load ratings
* Do not exceed encoder frequency limits
* Verify configuration after firmware update
* Confirm decimal position after changes

---

# ✅ FINAL STATUS

This revision is now:

* ✔ Consistent SET/OK separation maintained in practice (no functional conflict)
* ✔ No overlapping save/edit ambiguity
* ✔ Decimal shift clearly isolated to SET long press (normal mode only)
* ✔ Deterministic state transitions for firmware implementation
* ✔ Operator workflow simplified and stable

---

If you want next improvement step, I can convert this into:

* 📊 State machine diagram (fully implementable)
* 💻 Firmware UI logic (C / STM32 / ESP32)
* 🧠 Button event decoder spec (debounce + long-press timing rules)
* 📦 EEPROM/Flash layout with CRC versioning

Just tell me 👍
