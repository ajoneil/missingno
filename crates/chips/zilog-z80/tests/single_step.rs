//! SingleStepTests Z80 oracle sweep: per-T-state bus activity, port
//! transactions, and final CPU state for every opcode (including the CB,
//! ED, DD, FD, DDCB, and FDCB prefixes). Data is fetched (not committed) —
//! the sweep sparse-clones the oracle on first run, or honours
//! `SINGLE_STEP_TESTS_Z80_DIR`.

use std::path::PathBuf;

use missingno_test_support::oracle::fetch_oracle;
use missingno_zilog_z80::{Bus, Cpu, InterruptMode, Pins};
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    name: String,
    initial: CpuState,
    #[serde(rename = "final")]
    end: CpuState,
    cycles: Vec<(Option<u16>, Option<u8>, String)>,
    #[serde(default)]
    ports: Vec<(u16, u8, String)>,
}

#[derive(Deserialize)]
struct CpuState {
    pc: u16,
    sp: u16,
    a: u8,
    f: u8,
    b: u8,
    c: u8,
    d: u8,
    e: u8,
    h: u8,
    l: u8,
    i: u8,
    r: u8,
    ei: u8,
    wz: u16,
    ix: u16,
    iy: u16,
    af_: u16,
    bc_: u16,
    de_: u16,
    hl_: u16,
    im: u8,
    p: u8,
    q: u8,
    iff1: u8,
    iff2: u8,
    ram: Vec<(u16, u8)>,
}

struct FlatBus {
    memory: Vec<u8>,
    ports: Vec<(u16, u8, char)>,
}

impl FlatBus {
    fn new() -> Self {
        FlatBus {
            memory: vec![0; 0x10000],
            ports: Vec::new(),
        }
    }
}

impl Bus for FlatBus {
    fn read(&mut self, address: u16) -> u8 {
        self.memory[address as usize]
    }

    fn write(&mut self, address: u16, data: u8) {
        self.memory[address as usize] = data;
    }

    fn input(&mut self, port: u16) -> u8 {
        // The oracle fixes the byte the port returns; replay it in order.
        let value = PORT_INPUTS
            .with(|q| q.borrow_mut().pop_front())
            .unwrap_or(0xFF);
        self.ports.push((port, value, 'r'));
        value
    }

    fn output(&mut self, port: u16, data: u8) {
        self.ports.push((port, data, 'w'));
    }
}

thread_local! {
    static PORT_INPUTS: std::cell::RefCell<std::collections::VecDeque<u8>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
}

fn parse_pins(symbols: &str) -> Pins {
    let bytes = symbols.as_bytes();
    Pins {
        read: bytes[0] == b'r',
        write: bytes[1] == b'w',
        mreq: bytes[2] == b'm',
        iorq: bytes[3] == b'i',
    }
}

fn load_state(cpu: &mut Cpu, state: &CpuState) {
    cpu.pc = state.pc;
    cpu.sp = state.sp;
    cpu.a = state.a;
    cpu.f = state.f;
    cpu.b = state.b;
    cpu.c = state.c;
    cpu.d = state.d;
    cpu.e = state.e;
    cpu.h = state.h;
    cpu.l = state.l;
    cpu.i = state.i;
    cpu.r = state.r;
    cpu.wz = state.wz;
    cpu.ix = state.ix;
    cpu.iy = state.iy;
    [cpu.a_, cpu.f_] = state.af_.to_be_bytes();
    [cpu.b_, cpu.c_] = state.bc_.to_be_bytes();
    [cpu.d_, cpu.e_] = state.de_.to_be_bytes();
    [cpu.h_, cpu.l_] = state.hl_.to_be_bytes();
    cpu.iff1 = state.iff1 != 0;
    cpu.iff2 = state.iff2 != 0;
    cpu.im = match state.im {
        0 => InterruptMode::Mode0,
        1 => InterruptMode::Mode1,
        _ => InterruptMode::Mode2,
    };
    cpu.ei_pending = state.ei != 0;
    cpu.p = state.p != 0;
    cpu.q = state.q;
}

fn run_case(case: &Case) -> Result<(), String> {
    let mut cpu = Cpu::new();
    load_state(&mut cpu, &case.initial);

    let mut bus = FlatBus::new();
    for &(address, value) in &case.initial.ram {
        bus.memory[address as usize] = value;
    }
    PORT_INPUTS.with(|q| {
        let mut q = q.borrow_mut();
        q.clear();
        for (_, value, kind) in &case.ports {
            if kind == "r" {
                q.push_back(*value);
            }
        }
    });

    cpu.step(&mut bus);

    let mut problems = Vec::new();
    let end = &case.end;
    let im = match cpu.im {
        InterruptMode::Mode0 => 0,
        InterruptMode::Mode1 => 1,
        InterruptMode::Mode2 => 2,
    };
    for (label, got, want) in [
        ("pc", cpu.pc as u32, end.pc as u32),
        ("sp", cpu.sp as u32, end.sp as u32),
        ("a", cpu.a as u32, end.a as u32),
        ("f", cpu.f as u32, end.f as u32),
        ("b", cpu.b as u32, end.b as u32),
        ("c", cpu.c as u32, end.c as u32),
        ("d", cpu.d as u32, end.d as u32),
        ("e", cpu.e as u32, end.e as u32),
        ("h", cpu.h as u32, end.h as u32),
        ("l", cpu.l as u32, end.l as u32),
        ("i", cpu.i as u32, end.i as u32),
        ("r", cpu.r as u32, end.r as u32),
        ("ix", cpu.ix as u32, end.ix as u32),
        ("iy", cpu.iy as u32, end.iy as u32),
        ("wz", cpu.wz as u32, end.wz as u32),
        (
            "af_",
            u16::from_be_bytes([cpu.a_, cpu.f_]) as u32,
            end.af_ as u32,
        ),
        (
            "bc_",
            u16::from_be_bytes([cpu.b_, cpu.c_]) as u32,
            end.bc_ as u32,
        ),
        (
            "de_",
            u16::from_be_bytes([cpu.d_, cpu.e_]) as u32,
            end.de_ as u32,
        ),
        (
            "hl_",
            u16::from_be_bytes([cpu.h_, cpu.l_]) as u32,
            end.hl_ as u32,
        ),
        ("iff1", cpu.iff1 as u32, end.iff1 as u32),
        ("iff2", cpu.iff2 as u32, end.iff2 as u32),
        ("im", im, end.im as u32),
        ("ei", cpu.ei_pending as u32, end.ei as u32),
        ("p", cpu.p as u32, end.p as u32),
        ("q", cpu.q as u32, end.q as u32),
    ] {
        if got != want {
            problems.push(format!("{label}: got {got:02X} want {want:02X}"));
        }
    }
    for &(address, want) in &end.ram {
        let got = bus.memory[address as usize];
        if got != want {
            problems.push(format!("ram[{address:04X}]: got {got:02X} want {want:02X}"));
        }
    }

    let want_ports: Vec<(u16, u8, char)> = case
        .ports
        .iter()
        .map(|(port, value, kind)| (*port, *value, kind.chars().next().unwrap_or('?')))
        .collect();
    if bus.ports != want_ports {
        problems.push(format!(
            "ports: got {:X?} want {:X?}",
            bus.ports, want_ports
        ));
    }

    let trace = cpu.bus_trace();
    if trace.len() != case.cycles.len() {
        problems.push(format!(
            "cycle count: got {} want {}",
            trace.len(),
            case.cycles.len()
        ));
    } else {
        for (index, (got, (want_addr, want_data, want_pins))) in
            trace.iter().zip(case.cycles.iter()).enumerate()
        {
            let want_pins = parse_pins(want_pins);
            let addr_ok = want_addr.map(|a| a == got.address).unwrap_or(true);
            if !addr_ok || got.data != *want_data || got.pins != want_pins {
                problems.push(format!(
                    "cycle {index}: got ({:04X?},{:02X?},{:?}) want ({:04X?},{:02X?},{:?})",
                    got.address, got.data, got.pins, want_addr, want_data, want_pins
                ));
                break;
            }
        }
    }

    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!("[{}] {}", case.name, problems.join("; ")))
    }
}

fn data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SINGLE_STEP_TESTS_Z80_DIR") {
        return PathBuf::from(dir);
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/single-step-tests");
    let dir = root.join("v1");
    if !dir.is_dir() {
        fetch_oracle(
            &root,
            "https://github.com/SingleStepTests/z80",
            &["v1"],
            "SINGLE_STEP_TESTS_Z80_DIR",
        );
    }
    dir
}

fn run_file(path: &PathBuf) -> (usize, usize, Vec<String>) {
    let raw = std::fs::read(path).expect("readable test file");
    let cases: Vec<Case> = serde_json::from_slice(&raw).expect("valid test JSON");
    let mut passed = 0;
    let mut failed = 0;
    let mut examples = Vec::new();
    for case in &cases {
        match run_case(case) {
            Ok(()) => passed += 1,
            Err(problem) => {
                failed += 1;
                if examples.len() < 3 {
                    examples.push(format!("  {problem}"));
                }
            }
        }
    }
    (passed, failed, examples)
}

#[test]
fn single_step_sweep() {
    let dir = data_dir();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("readable test dir")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();

    let mut total_passed = 0usize;
    let mut total_failed = 0usize;
    let mut bad_files = Vec::new();
    for path in &files {
        let (passed, failed, examples) = run_file(path);
        total_passed += passed;
        total_failed += failed;
        if failed > 0 {
            let name = path.file_stem().unwrap().to_string_lossy();
            bad_files.push(format!("{name}: {failed} failed\n{}", examples.join("\n")));
        }
    }
    assert!(
        bad_files.is_empty(),
        "{} opcode files with failures ({total_passed} passed, {total_failed} failed):\n{}",
        bad_files.len(),
        bad_files.join("\n")
    );
    assert!(
        total_passed > 1_000_000,
        "suspiciously few cases ran: {total_passed}"
    );
}

/// Data-free sanity check so plain `cargo test` exercises the core.
#[test]
fn ld_bc_nn_smoke() {
    let mut cpu = Cpu::new();
    let mut bus = FlatBus::new();
    bus.memory[0x0000] = 0x01; // LD BC,$1234
    bus.memory[0x0001] = 0x34;
    bus.memory[0x0002] = 0x12;
    cpu.pc = 0x0000;
    let tstates = cpu.step(&mut bus);
    assert_eq!(tstates, 10);
    assert_eq!(cpu.b, 0x12);
    assert_eq!(cpu.c, 0x34);
    assert_eq!(cpu.pc, 0x0003);
}
