/* QEMU mps2-an505 boots SECURE: use the secure aliases —
   code SSRAM at 0x10000000, SSRAM2/3 at 0x38000000. */
MEMORY
{
  FLASH : ORIGIN = 0x10000000, LENGTH = 4M
  RAM   : ORIGIN = 0x38000000, LENGTH = 4M
}
