#!/bin/sh
# Assemble the timer probes with RGBDS.
cd "$(dirname "$0")" || exit 1
for r in *.asm; do
  b=${r%.asm}
  rgbasm -o "$b.o" "$r" && rgblink -o "$b.gb" "$b.o" && rgbfix -v -p 0 "$b.gb" && rm "$b.o"
done
