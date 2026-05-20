# GBC-only Test ROM Attribution

CGB-only test ROMs from the [c-sp/game-boy-test-roms](https://github.com/c-sp/game-boy-test-roms)
collection (v7.0). DMG-compatible ROMs live in `crates/missingno-gb/tests/accuracy/roms/`.

## cgb-acid2

- **Author:** Matt Currie
- **Source:** https://github.com/mattcurrie/cgb-acid2
- **License:** MIT
- **Purpose:** CGB PPU correctness — palettes, tile attributes, BG/OBJ priority.

## cgb-acid-hell

- **Author:** Matt Currie
- **Source:** https://github.com/mattcurrie/cgb-acid-hell
- **License:** MIT
- **Purpose:** More demanding CGB PPU edge cases than cgb-acid2.

## rtc3test

- **Author:** ax6 (aaaaaa123456789)
- **Source:** https://github.com/aaaaaa123456789/rtc3test
- **License:** MIT
- **Purpose:** MBC3 real-time clock correctness. Device-agnostic but lives
  here as it isn't currently in the DMG suite either.
