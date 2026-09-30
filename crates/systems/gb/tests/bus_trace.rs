//! What a host sees of each instruction: its bus accesses, stamped with the
//! master-clock edge they were committed on, and the interrupt dispatches
//! between instructions.

use missingno_gb::GameBoy;
use missingno_gb::cartridge::Cartridge;
use missingno_gb::cpu_bus::BusAccessKind;

/// LD SP,$FFFE; LD A,1; LDH (IE),A; LDH (IF),A; EI; NOP; NOP; JR -2, with
/// LD A,$12; LDH ($80),A; RETI at the VBlank vector.
fn console() -> GameBoy {
    let mut rom = vec![0u8; 0x8000];
    rom[0x100..0x10e].copy_from_slice(&[
        0x31, 0xfe, 0xff, 0x3e, 0x01, 0xe0, 0xff, 0xe0, 0x0f, 0xfb, 0x00, 0x00, 0x18, 0xfe,
    ]);
    rom[0x40..0x45].copy_from_slice(&[0x3e, 0x12, 0xe0, 0x80, 0xd9]);
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
fn each_access_carries_the_master_edge_it_was_committed_on() {
    let mut gb = console();
    run_to(&mut gb, 0x105);
    let before = gb.master_edge();
    let result = gb.step_recorded();
    let after = gb.master_edge();
    assert_eq!(after - before, 2 * u64::from(result.tcycles));

    let accesses: Vec<_> = gb.bus_trace().iter().map(|a| (a.address, a.kind)).collect();
    assert_eq!(
        accesses,
        [
            (0x105, BusAccessKind::Read),
            (0x106, BusAccessKind::Read),
            (0xffff, BusAccessKind::Write),
        ]
    );
    let edges: Vec<u64> = gb.bus_trace().iter().map(|a| a.master_edge).collect();
    assert!(edges.iter().all(|e| (before..after).contains(e)));
    assert!(
        edges.windows(2).all(|w| w[1] - w[0] == 8),
        "one M-cycle apart: {edges:?}"
    );
}

#[test]
fn a_dispatch_is_seen_before_the_step_that_pushes_the_return_address() {
    let mut gb = console();
    run_to(&mut gb, 0x109);
    let mut steps = 0;
    while !gb.cpu().in_dispatch() {
        gb.step();
        steps += 1;
        assert!(steps < 10, "the VBlank interrupt was never dispatched");
    }
    let sp = gb.cpu().stack_pointer;
    let return_address = gb.cpu().ir_address;
    gb.step_recorded();

    let writes: Vec<_> = gb
        .bus_trace()
        .iter()
        .filter(|a| a.kind == BusAccessKind::Write)
        .map(|a| (a.address, a.value))
        .collect();
    let [low, high] = return_address.to_le_bytes();
    assert_eq!(
        writes,
        [(sp.wrapping_sub(1), high), (sp.wrapping_sub(2), low)]
    );
    assert_eq!(gb.cpu().ir_address, 0x40);
    assert!(!gb.cpu().in_dispatch());
}
