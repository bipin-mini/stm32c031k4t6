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

  /* Symbols for optional manual copy */
  _sramfunc = LOADADDR(.ramfunc);
  _eramfunc = _sramfunc + SIZEOF(.ramfunc);
}