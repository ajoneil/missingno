//! What an observer sees of a run: the shadow call stack, routine entries,
//! the watched instructions and accesses, and the SGB's commands.

use missingno_gb::GameBoy;
use missingno_gb::cartridge::Cartridge;
use missingno_gb::observe::{AccessKind, Doing, Event, Filters, Observer, OpenCall, Opened};

/// LD SP,$FFFE; CALL $0200; LD A,$12; LD ($C000),A; JP $0300, with
/// LD A,$34; LDH ($80),A; RET at $0200 and NOP; JR -2 at $0300.
fn console() -> GameBoy {
    let mut rom = vec![0u8; 0x8000];
    rom[0x100..0x10e].copy_from_slice(&[
        0x31, 0xfe, 0xff, 0xcd, 0x00, 0x02, 0x3e, 0x12, 0xea, 0x00, 0xc0, 0xc3, 0x00, 0x03,
    ]);
    rom[0x200..0x205].copy_from_slice(&[0x3e, 0x34, 0xe0, 0x80, 0xc9]);
    rom[0x300..0x303].copy_from_slice(&[0x00, 0x18, 0xfd]);
    GameBoy::new(Cartridge::new(rom, None, None).unwrap(), None)
}

fn run_to(gb: &mut GameBoy, address: u16) {
    for _ in 0..1000 {
        if gb.cpu().ir_address == address {
            return;
        }
        gb.step();
    }
    panic!("never reached {address:04x}");
}

#[test]
fn a_call_opens_and_its_return_closes_it() {
    let mut gb = console();
    gb.set_observer(Some(Observer::new(Filters::default())));
    run_to(&mut gb, 0x200);
    let call = OpenCall {
        opened: Opened::Call { site: 0x103 },
        pc: 0x200,
        bank: 0,
        slot: 0xfffc,
    };
    assert_eq!(gb.call_stack(), [call]);
    run_to(&mut gb, 0x106);
    assert_eq!(gb.call_stack(), []);
    let events = gb.observer_mut().unwrap().take_events();
    assert!(
        matches!(events[..], [Event::Opened { call: c, .. }, Event::Closed { .. }] if c == call)
    );
}

#[test]
fn coming_to_an_entry_by_a_jump_enters_it() {
    let mut gb = console();
    let filters = Filters {
        entries: [(0, 0x300)].into(),
        ..Filters::default()
    };
    gb.set_observer(Some(Observer::new(filters)));
    run_to(&mut gb, 0x300);
    gb.step();
    gb.step();
    let entry = OpenCall {
        opened: Opened::Entry,
        pc: 0x300,
        bank: 0,
        slot: 0xfffe,
    };
    assert_eq!(
        gb.call_stack(),
        [entry],
        "looping back to it enters it once"
    );
}

#[test]
fn watched_writes_are_reported_and_an_instructions_own_bytes_are_not_reads() {
    let mut gb = console();
    let mut filters = Filters::default();
    filters.write.insert(0xc000);
    filters.read.insert_range(0x100, 0x110);
    filters.exec.insert(0x108);
    gb.set_observer(Some(Observer::new(filters)));
    run_to(&mut gb, 0x10b);
    let events = gb.observer_mut().unwrap().take_events();
    let accesses: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            Event::Access(a) => Some((a.kind, a.address, a.value, a.pc, a.doing)),
            _ => None,
        })
        .collect();
    assert_eq!(
        accesses,
        [(AccessKind::Write, 0xc000, 0x12, 0x108, Doing::Instruction)]
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Executed { pc: 0x108, .. }))
    );
}

#[test]
fn an_interrupt_opens_a_call_its_reti_closes() {
    // LD SP,$FFFE; LD A,1; LDH (IE),A; LDH (IF),A; EI; NOP; NOP; JR -2, with
    // RETI at the VBlank vector.
    let mut rom = vec![0u8; 0x8000];
    rom[0x100..0x10e].copy_from_slice(&[
        0x31, 0xfe, 0xff, 0x3e, 0x01, 0xe0, 0xff, 0xe0, 0x0f, 0xfb, 0x00, 0x00, 0x18, 0xfe,
    ]);
    rom[0x40] = 0xd9;
    let mut gb = GameBoy::new(Cartridge::new(rom, None, None).unwrap(), None);
    gb.set_observer(Some(Observer::new(Filters::default())));
    run_to(&mut gb, 0x40);
    assert!(matches!(
        gb.call_stack(),
        [OpenCall {
            opened: Opened::Interrupt { .. },
            pc: 0x40,
            ..
        }]
    ));
    gb.step();
    assert_eq!(gb.call_stack(), []);
    let events = gb.observer_mut().unwrap().take_events();
    assert!(matches!(events[0], Event::Dispatching { .. }));
}

#[test]
fn a_restored_machine_has_no_calls_open() {
    let mut gb = console();
    gb.set_observer(Some(Observer::new(Filters::default())));
    run_to(&mut gb, 0x200);
    gb.reset();
    assert_eq!(gb.call_stack(), []);
}

#[test]
fn the_sgb_commands_are_kept_while_asked_for() {
    // An SGB cartridge sending MASK_EN(1): JOYP writes, bit by bit.
    let mut code = vec![0x31, 0xfe, 0xff];
    let mut joyp = |v: u8| code.extend([0x3e, v, 0xe0, 0x00]);
    joyp(0x00);
    joyp(0x30);
    let packet = [0xb9u8, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    for byte in packet {
        for bit in 0..8 {
            joyp(if byte >> bit & 1 != 0 { 0x10 } else { 0x20 });
            joyp(0x30);
        }
    }
    joyp(0x20);
    joyp(0x30);
    code.extend([0x18, 0xfe]);
    let mut rom = vec![0u8; 0x8000];
    rom[0x100..0x104].copy_from_slice(&[0x00, 0xc3, 0x50, 0x01]);
    rom[0x150..0x150 + code.len()].copy_from_slice(&code);
    (rom[0x146], rom[0x14b]) = (0x03, 0x33);
    let mut gb = GameBoy::new(Cartridge::new(rom, None, None).unwrap(), None);
    gb.observe_sgb_commands(true);
    let end = 0x150 + code.len() as u16 - 2;
    for _ in 0..10_000 {
        if gb.cpu().ir_address == end {
            break;
        }
        gb.step();
    }
    assert_eq!(gb.take_sgb_commands(), [packet.to_vec()]);
    assert_eq!(gb.take_sgb_commands(), Vec::<Vec<u8>>::new());
}

#[test]
fn the_observer_counts_the_pictures_presented() {
    let mut gb = console();
    gb.set_observer(Some(Observer::new(Filters::default())));
    let mut presented = 0;
    for _ in 0..60_000 {
        presented += gb.step().new_screen as u64;
    }
    assert!(presented > 0);
    assert_eq!(gb.observer_mut().unwrap().presented(), presented);
}
