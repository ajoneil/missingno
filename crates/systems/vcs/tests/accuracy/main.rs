// Test function names mirror their ROM filenames (with region suffix) so a
// failure points straight at the ROM. Self-tests read the RESULT RAM
// convention; screenshot tests diff against the _<region>.png reference.
#![allow(non_snake_case)]

mod cartridge;
mod collision;
mod common;
mod cpu;
mod harness;
mod recording;
mod riot;
mod savestate;
mod tia_render;
mod tia_timing;
