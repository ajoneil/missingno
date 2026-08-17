//! Board smoke tier: the ti-vdp conformance corpus run through the whole
//! console rather than the chip crate's testbench, plus the two board
//! behaviours that testbench has no wiring for — the joystick multiplexers
//! and the PSG's READY→/WAIT stall.

use missingno_core::ports::PortId;
use missingno_core::system::{ControlId, ControlInput, ControlRole, ControlSite};
use missingno_sg1000::console::{Sg1000, TSTATES_PER_FRAME};
use missingno_test_support::verdict::{Outcome, Poll, poll_verdict};

/// The chip crate's corpus: the same self-checking `.sg` ROMs, driven here by
/// a real board map instead of the testbench's common-subset envelope.
const CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../chips/ti-vdp/tests/accuracy/roms/"
);

/// Each ROM latches a verdict into the RESULT block at the base of work RAM.
const RESULT_BLOCK: u16 = 0xC000;

/// The chip crate's default: the timing sweeps run ~550 frames to a verdict.
const BUDGET_FRAMES: u64 = 1200;
/// No Z80 instruction is shorter than four T-states.
const INSTRUCTION_BUDGET: u64 = BUDGET_FRAMES * TSTATES_PER_FRAME as u64 / 4;

/// Run a corpus ROM to its verdict on the console, panicking on FAIL.
fn run_to_verdict(rom: &str) -> Sg1000 {
    let path = format!("{CORPUS}{rom}");
    let image = std::fs::read(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"));
    let mut console = Sg1000::new(&image, None).expect("flat cartridge image");
    let outcome = poll_verdict(INSTRUCTION_BUDGET, || {
        console.step_instruction();
        Poll::Read([0, 1, 2, 3].map(|offset| console.peek(RESULT_BLOCK + offset)))
    });

    match outcome {
        Outcome::Reached(verdict) if verdict.passed => console,
        Outcome::Reached(verdict) => panic!(
            "{rom}: FAIL code={:02X} observed={:02X} expected={:02X}",
            verdict.code, verdict.observed, verdict.expected
        ),
        _ => panic!("{rom}: no verdict within {BUDGET_FRAMES} frames"),
    }
}

/// The 1 KB work RAM repeating to $FFFF — the board's own decode, and the
/// substrate of the documented machine-detection routine.
#[test]
fn work_ram_mirrors_through_the_top_window() {
    run_to_verdict("harness/ram-mirror.sg");
}

/// The VDP answering across the whole $80-$BF block, A0 selecting the port.
#[test]
fn vdp_ports_alias_across_their_block() {
    run_to_verdict("harness/port-mirror.sg");
}

/// The frame flag against the interrupt line, with the CPU walking the
/// crystal grid one T-state at a time.
#[test]
fn frame_flag_races_resolve_on_the_board() {
    run_to_verdict("timing/f-race.sg");
}

/// A rendered scene reaches the frame the console hands out.
#[test]
fn a_graphics_scene_reaches_a_non_blank_frame() {
    let mut console = run_to_verdict("modes/graphic1.sg");
    let frame = console
        .step_frame(2 * TSTATES_PER_FRAME)
        .expect("a frame completes once the scene is up");
    let lit = frame.pixels.iter().filter(|&&index| index != 0).count();
    assert!(lit > 0, "the scene renders something");
}

struct Asm {
    bytes: Vec<u8>,
}

impl Asm {
    fn new() -> Self {
        Asm { bytes: Vec::new() }
    }
    fn here(&self) -> u16 {
        self.bytes.len() as u16
    }
    fn emit(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }
    fn ld_a(&mut self, value: u8) {
        self.emit(&[0x3E, value]);
    }
    fn in_port(&mut self, port: u8) {
        self.emit(&[0xDB, port]);
    }
    fn out_port(&mut self, port: u8) {
        self.emit(&[0xD3, port]);
    }
    fn ld_addr_a(&mut self, address: u16) {
        self.emit(&[0x32, address as u8, (address >> 8) as u8]);
    }
    fn jp(&mut self, target: u16) {
        self.emit(&[0xC3, target as u8, (target >> 8) as u8]);
    }
    /// Pad to the smallest flat image the board mirrors through `/EXM2`.
    fn into_rom(mut self) -> Vec<u8> {
        self.bytes.resize(0x8000, 0);
        self.bytes
    }
}

fn press(console: &mut Sg1000, port: PortId, role: ControlRole) {
    console.apply_control(
        ControlId {
            site: ControlSite::Port(port),
            role,
        },
        ControlInput::Digital(true),
    );
}

/// Read the four conventional joystick addresses into work RAM.
fn joystick_probe() -> Vec<u8> {
    let mut asm = Asm::new();
    for (index, port) in [0xDCu8, 0xDD, 0xDE, 0xDF].into_iter().enumerate() {
        asm.in_port(port);
        asm.ld_addr_a(0xC010 + index as u16);
    }
    let spin = asm.here();
    asm.jp(spin);
    asm.into_rom()
}

fn run_joystick_probe(console: &mut Sg1000) -> [u8; 4] {
    for _ in 0..16 {
        console.step_instruction();
    }
    [0, 1, 2, 3].map(|offset| console.peek(0xC010 + offset))
}

#[test]
fn released_joysticks_read_all_ones() {
    let mut console = Sg1000::new(&joystick_probe(), None).unwrap();
    assert_eq!(run_joystick_probe(&mut console), [0xFF; 4]);
}

/// Enri's map, active low: player 1's six lines and player 2's two vertical
/// ones in $DC, player 2's remaining four in $DD.
#[test]
fn each_pad_line_clears_its_own_bit() {
    let cases = [
        (PortId(0), ControlRole::Up, 0xFEu8, 0xFFu8),
        (PortId(0), ControlRole::Down, 0xFD, 0xFF),
        (PortId(0), ControlRole::Left, 0xFB, 0xFF),
        (PortId(0), ControlRole::Right, 0xF7, 0xFF),
        (PortId(0), ControlRole::Action(0), 0xEF, 0xFF),
        (PortId(0), ControlRole::Action(1), 0xDF, 0xFF),
        (PortId(1), ControlRole::Up, 0xBF, 0xFF),
        (PortId(1), ControlRole::Down, 0x7F, 0xFF),
        (PortId(1), ControlRole::Left, 0xFF, 0xFE),
        (PortId(1), ControlRole::Right, 0xFF, 0xFD),
        (PortId(1), ControlRole::Action(0), 0xFF, 0xFB),
        (PortId(1), ControlRole::Action(1), 0xFF, 0xF7),
    ];
    for (port, role, dc, dd) in cases {
        let mut console = Sg1000::new(&joystick_probe(), None).unwrap();
        press(&mut console, port, role);
        let read = run_joystick_probe(&mut console);
        assert_eq!(
            (read[0], read[1]),
            (dc, dd),
            "port {} role {role:?}",
            port.0
        );
    }
}

/// Only A0 reaches the multiplexers, so the pair repeats every two addresses
/// through the whole $C0-$FF block.
#[test]
fn the_multiplexer_pair_aliases_every_two_addresses() {
    let mut console = Sg1000::new(&joystick_probe(), None).unwrap();
    press(&mut console, PortId(0), ControlRole::Left);
    press(&mut console, PortId(1), ControlRole::Right);
    let read = run_joystick_probe(&mut console);
    assert_eq!(read[2], read[0], "$DE reads as $DC");
    assert_eq!(read[3], read[1], "$DF reads as $DD");
}

/// `CON` and the three unconnected multiplexer inputs are tied high, so no
/// control can pull $DD's top nibble down.
#[test]
fn the_top_nibble_of_the_second_byte_stays_high() {
    let mut console = Sg1000::new(&joystick_probe(), None).unwrap();
    for port in [PortId(0), PortId(1)] {
        for role in [
            ControlRole::Up,
            ControlRole::Down,
            ControlRole::Left,
            ControlRole::Right,
            ControlRole::Action(0),
            ControlRole::Action(1),
        ] {
            press(&mut console, port, role);
        }
    }
    let read = run_joystick_probe(&mut console);
    assert_eq!(read[1] & 0xF0, 0xF0, "$DD bits 4-7 are hard ones");
    assert_eq!(read[0], 0x00, "every $DC line pulled down");
    assert_eq!(read[1] & 0x0F, 0x00, "every $DD line pulled down");
}

/// The PSG's READY drops for the clocks a byte takes to load, and it sits on
/// the Z80's /WAIT — so the OUT that fed it is the cycle that stretches.
#[test]
fn a_psg_write_stretches_its_own_out() {
    let mut asm = Asm::new();
    asm.ld_a(0x9F);
    asm.out_port(0x00); // the unused block: nothing answers, nothing stalls
    asm.ld_a(0x90); // channel 0 attenuation, wide open
    asm.out_port(0x7F);
    let spin = asm.here();
    asm.jp(spin);

    let mut console = Sg1000::new(&asm.into_rom(), None).unwrap();
    console.step_instruction();
    console.step_instruction();
    let unstalled = console.cpu.bus_trace().len();
    console.step_instruction();
    console.step_instruction();
    let stalled = console.cpu.bus_trace().len();

    assert_eq!(unstalled, 11, "OUT (n),A is eleven T-states");
    assert!(
        (28..=36).contains(&(stalled - unstalled)),
        "the PSG load costs about 32 extra T-states, got {}",
        stalled - unstalled
    );
    assert_eq!(
        console.psg().attenuations()[0],
        0,
        "the byte reached the chip"
    );
}
