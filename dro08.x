/* ---------------------------------------------------------
   Use cortex-m-rt default linker script
--------------------------------------------------------- */
INCLUDE link.x


/* ---------------------------------------------------------
   Additional sections (safe extensions)
--------------------------------------------------------- */
SECTIONS
{
  /* ---------------- RAM EXECUTABLE CODE ---------------- */
  .ramfunc : ALIGN(4)
  {
    *(.ramfunc*)
  } > RAM AT > FLASH

  /* ---------------- REQUIRED SYMBOLS ---------------- */
  _ramfunc  = ADDR(.ramfunc);        /* RAM destination */
  _sramfunc = LOADADDR(.ramfunc);    /* FLASH source */
  _eramfunc = _sramfunc + SIZEOF(.ramfunc);
}