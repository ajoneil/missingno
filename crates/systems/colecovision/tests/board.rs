//! What the corpus never exercises: the controller mode latch and both
//! segments of each connector, the PSG's READY and the M1 wait on /WAIT, and
//! the Reset button leaving the VDP alone. Each probe is a test cartridge the
//! real BIOS jumps straight into, so every test reads `COLECOVISION_BIOS` and
//! skips without it.

use missingno_colecovision::console::{ColecoVision, PORT1, PORT2};
use missingno_colecovision::controllers::ControllerMode;
use missingno_colecovision::firmware::BIOS_SIZE;
use missingno_core::ports::PortId;
use missingno_core::system::{ControlId, ControlInput, ControlRole, ControlSite};
use missingno_test_support::asm::Z80Asm;
use missingno_ti_vdp::Standard;

/// Where the cartridge's first select maps it.
const CART_BASE: u16 = 0x8000;
/// `START_GAME`, the header's entry pointer.
const START_GAME: u16 = 0x0A;
/// The eight 3-byte jump slots for RST $08-$30, the IRQ and the NMI.
const VECTOR_SLOTS: usize = 8;
/// Where the probes latch what they read.
const PROBE_RAM: u16 = 0x7010;

fn bios(test: &str) -> Option<[u8; BIOS_SIZE]> {
    let Some(path) = std::env::var_os("COLECOVISION_BIOS") else {
        println!("{test}: skipped — set COLECOVISION_BIOS");
        return None;
    };
    let bytes = std::fs::read(&path).expect("the BIOS image reads");
    Some(bytes.try_into().expect("an 8 KB BIOS image"))
}

/// A test cartridge: the `$55,$AA` magic the BIOS jumps straight past, the
/// entry pointer, every vector slot a `RET`, then `body` at the entry. Returns
/// the image and the entry address.
fn cartridge(body: impl FnOnce(&mut Z80Asm, u16)) -> (Vec<u8>, u16) {
    let mut asm = Z80Asm::new();
    asm.emit(&[0x55, 0xAA]);
    asm.pad_to(START_GAME);
    let entry = CART_BASE + START_GAME + 2 + 3 * VECTOR_SLOTS as u16;
    asm.emit(&entry.to_le_bytes());
    for _ in 0..VECTOR_SLOTS {
        asm.ret();
        asm.emit(&[0x00, 0x00]);
    }
    assert_eq!(CART_BASE + asm.here(), entry);
    body(&mut asm, CART_BASE);
    (asm.into_rom(0x2000), entry)
}

fn console(test: &str, image: &[u8]) -> Option<ColecoVision> {
    let bios = bios(test)?;
    Some(ColecoVision::new(image, Standard::Ntsc, bios).expect("a flat cartridge image"))
}

/// Step until the CPU reaches `pc`, as the BIOS hands over.
fn run_to(console: &mut ColecoVision, pc: u16) {
    for _ in 0..10_000 {
        if console.cpu.pc == pc {
            return;
        }
        console.step_instruction();
    }
    panic!("the CPU never reached ${pc:04X}");
}

/// The switches held through a probe run.
type Held = &'static [(PortId, ControlRole)];

/// Select the joystick segment, read J5 then J6; select the keypad segment,
/// read both again; spin.
fn controller_probe() -> (Vec<u8>, u16) {
    let mut spin = 0;
    let (image, _) = cartridge(|asm, base| {
        asm.out_port(0xC0);
        asm.in_port(0xFC);
        asm.ld_addr_a(PROBE_RAM);
        asm.in_port(0xFF);
        asm.ld_addr_a(PROBE_RAM + 1);
        asm.out_port(0x80);
        asm.in_port(0xFC);
        asm.ld_addr_a(PROBE_RAM + 2);
        asm.in_port(0xFF);
        asm.ld_addr_a(PROBE_RAM + 3);
        spin = base + asm.here();
        asm.jp(spin);
    });
    (image, spin)
}

/// $FC and $FF on the joystick segment, then on the keypad segment.
fn read_controllers(test: &str, held: &[(PortId, ControlRole)]) -> Option<[u8; 4]> {
    let (image, spin) = controller_probe();
    let mut console = console(test, &image)?;
    for &(port, role) in held {
        console.apply_control(ControlId::port(port, role), ControlInput::Digital(true));
    }
    run_to(&mut console, spin);
    Some([0, 1, 2, 3].map(|offset| console.peek(PROBE_RAM + offset)))
}

#[test]
fn released_controllers_read_all_ones_on_both_segments() {
    let Some(read) = read_controllers("released_controllers_read_all_ones_on_both_segments", &[])
    else {
        return;
    };
    assert_eq!(read, [0xFF; 4]);
}

/// Each switch reaches only its own connector and only the segment the mode
/// latch selects: [J5 joystick, J6 joystick, J5 keypad, J6 keypad].
#[test]
fn each_switch_reads_on_its_own_segment() {
    let cases: &[(Held, [u8; 4])] = &[
        (&[(PORT1, ControlRole::Up)], [0xFE, 0xFF, 0xFF, 0xFF]),
        (&[(PORT2, ControlRole::Up)], [0xFF, 0xFE, 0xFF, 0xFF]),
        (&[(PORT1, ControlRole::Action(0))], [0xBF, 0xFF, 0xFF, 0xFF]),
        (&[(PORT1, ControlRole::Action(1))], [0xFF, 0xFF, 0xBF, 0xFF]),
        (&[(PORT1, ControlRole::Key(4))], [0xFF, 0xFF, 0xF3, 0xFF]),
        (
            &[(PORT1, ControlRole::Key(0)), (PORT1, ControlRole::Key(1))],
            [0xFF, 0xFF, 0xF5, 0xFF],
        ),
    ];
    for (held, expected) in cases {
        let Some(read) = read_controllers("each_switch_reads_on_its_own_segment", held) else {
            return;
        };
        assert_eq!(read, *expected, "{held:?}");
    }
}

/// Every opcode fetch buys one wait state: `LD A,(nn)` is 13 T on a bare Z80
/// and 14 here.
#[test]
fn an_opcode_fetch_costs_one_wait_state() {
    let (image, entry) = cartridge(|asm, base| {
        asm.ld_a_addr(0x7000);
        let spin = base + asm.here();
        asm.jp(spin);
    });
    let Some(mut console) = console("an_opcode_fetch_costs_one_wait_state", &image) else {
        return;
    };
    run_to(&mut console, entry);
    console.step_instruction();
    assert_eq!(console.cpu.bus_trace().len(), 14);
}

/// The PSG's READY drops for the clocks a byte takes to load, and it shares
/// the pulled-up /WAIT line with the M1 latch — so the OUT that fed it is the
/// cycle that stretches.
#[test]
fn a_psg_write_stretches_its_own_out() {
    let (image, entry) = cartridge(|asm, base| {
        asm.ld_a(0x9F);
        asm.out_port(0x00); // no select: nothing answers, nothing stalls
        asm.ld_a(0x90); // channel 0 attenuation, wide open
        asm.out_port(0xFF);
        let spin = base + asm.here();
        asm.jp(spin);
    });
    let Some(mut console) = console("a_psg_write_stretches_its_own_out", &image) else {
        return;
    };
    run_to(&mut console, entry);
    console.step_instruction();
    console.step_instruction();
    let unstalled = console.cpu.bus_trace().len();
    console.step_instruction();
    console.step_instruction();
    let stalled = console.cpu.bus_trace().len();

    assert_eq!(
        unstalled, 12,
        "OUT (n),A is eleven T-states and one M1 wait"
    );
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

/// `CPU_RESET` reaches the Z80 and the two latches, not the VDP's /RESET: its
/// registers and raster position ride through the button.
#[test]
fn the_reset_button_leaves_the_vdp_alone() {
    let mut spin = 0;
    let (image, _) = cartridge(|asm, base| {
        asm.ld_a(0xF4);
        asm.out_port(0xBF);
        asm.ld_a(0x87); // R7: text and backdrop colours
        asm.out_port(0xBF);
        asm.out_port(0x80);
        spin = base + asm.here();
        asm.jp(spin);
    });
    let Some(mut console) = console("the_reset_button_leaves_the_vdp_alone", &image) else {
        return;
    };
    run_to(&mut console, spin);
    assert_eq!(console.vdp().registers()[7], 0xF4);
    assert_eq!(console.board_state().mode, ControllerMode::Keypad);
    let registers = *console.vdp().registers();
    let line = console.vdp().line();

    console.apply_control(
        ControlId {
            site: ControlSite::Panel,
            role: ControlRole::Reset,
        },
        ControlInput::Digital(true),
    );

    assert_eq!(console.cpu.pc, 0x0000);
    assert_eq!(console.board_state().mode, ControllerMode::Joystick);
    assert_eq!(*console.vdp().registers(), registers);
    assert_eq!(console.vdp().line(), line);
}
