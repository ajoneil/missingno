use std::path::PathBuf;
use std::process;

use missingno_core::debugger::Debugger;
use missingno_core::debugger::instructions::InstructionsIterator;
use missingno_core::game_boy::GameBoy;
use missingno_core::game_boy::cartridge::Cartridge;
use missingno_core::game_boy::cpu::flags::Flags;
use missingno_core::game_boy::cpu::instructions::Instruction;
use serde::Serialize;
use tiny_http::{Method, Response, StatusCode};

pub fn run(rom_path: Option<PathBuf>) {
    let rom_path = rom_path.unwrap_or_else(|| {
        eprintln!("error: --headless requires a ROM file");
        process::exit(1);
    });

    let rom_data = std::fs::read(&rom_path).unwrap_or_else(|e| {
        eprintln!("error: failed to read {}: {e}", rom_path.display());
        process::exit(1);
    });

    let save_path = rom_path.with_extension("sav");
    let save_data = std::fs::read(&save_path).ok();

    let cartridge = Cartridge::new(rom_data, save_data);
    let title = cartridge.title().to_string();
    let game_boy = GameBoy::new(cartridge);
    let mut debugger = Debugger::new(game_boy);

    let server = tiny_http::Server::http("127.0.0.1:3333").unwrap_or_else(|e| {
        eprintln!("error: failed to bind 127.0.0.1:3333: {e}");
        process::exit(1);
    });

    eprintln!("headless debugger ready: {title}");
    eprintln!("listening on http://127.0.0.1:3333");

    for request in server.incoming_requests() {
        handle_request(request, &mut debugger);
    }
}

fn handle_request(request: tiny_http::Request, debugger: &mut Debugger) {
    let method = request.method().clone();
    let path = request.url().to_string();

    match (&method, path.as_str()) {
        (&Method::Get, "/cpu") => {
            respond_json(request, cpu_state(debugger.game_boy()));
        }
        (&Method::Get, "/instructions") => {
            respond_json(request, disassemble(debugger.game_boy(), 20));
        }
        (&Method::Get, "/breakpoints") => {
            let addrs: Vec<String> = debugger
                .breakpoints()
                .iter()
                .map(|a| format!("{a:04x}"))
                .collect();
            respond_json(request, addrs);
        }
        (&Method::Post, "/step") => {
            debugger.step();
            respond_json(request, cpu_state(debugger.game_boy()));
        }
        (&Method::Post, "/step-frame") => {
            debugger.step_frame();
            respond_json(request, cpu_state(debugger.game_boy()));
        }
        (&Method::Post, "/step-over") => {
            debugger.step_over();
            respond_json(request, cpu_state(debugger.game_boy()));
        }
        (&Method::Post, "/reset") => {
            debugger.reset();
            respond_json(request, cpu_state(debugger.game_boy()));
        }
        _ if path.starts_with("/breakpoints/") => {
            let addr_str = &path["/breakpoints/".len()..];
            match u16::from_str_radix(addr_str, 16) {
                Ok(addr) => match &method {
                    &Method::Put => {
                        debugger.set_breakpoint(addr);
                        respond_json(request, serde_json::json!({ "set": format!("{addr:04x}") }));
                    }
                    &Method::Delete => {
                        debugger.clear_breakpoint(addr);
                        respond_json(
                            request,
                            serde_json::json!({ "cleared": format!("{addr:04x}") }),
                        );
                    }
                    _ => respond_error(request, 405, "method not allowed"),
                },
                Err(_) => respond_error(request, 400, "invalid hex address"),
            }
        }
        _ => respond_error(request, 404, "not found"),
    }
}

fn cpu_state(gb: &GameBoy) -> CpuState {
    let cpu = gb.cpu();
    CpuState {
        a: cpu.a,
        b: cpu.b,
        c: cpu.c,
        d: cpu.d,
        e: cpu.e,
        h: cpu.h,
        l: cpu.l,
        sp: cpu.stack_pointer,
        pc: cpu.program_counter,
        flags: FlagsState {
            zero: cpu.flags.contains(Flags::ZERO),
            negative: cpu.flags.contains(Flags::NEGATIVE),
            half_carry: cpu.flags.contains(Flags::HALF_CARRY),
            carry: cpu.flags.contains(Flags::CARRY),
        },
        ime: cpu.interrupts_enabled(),
        halted: cpu.halt_state != missingno_core::game_boy::cpu::HaltState::Running,
    }
}

fn disassemble(gb: &GameBoy, count: usize) -> Vec<InstructionEntry> {
    let pc = gb.cpu().program_counter;
    let mut it = InstructionsIterator::new(pc, gb);
    let mut entries = Vec::new();

    for _ in 0..count {
        if let Some(address) = it.address {
            if let Some(instruction) = Instruction::decode(&mut it) {
                entries.push(InstructionEntry {
                    address: format!("{address:04x}"),
                    text: instruction.to_string(),
                });
            } else {
                break;
            }
        }
    }

    entries
}

fn respond_json(request: tiny_http::Request, body: impl Serialize) {
    let json = serde_json::to_string_pretty(&body).unwrap();
    let response = Response::from_string(json)
        .with_header("Content-Type: application/json".parse::<tiny_http::Header>().unwrap());
    let _ = request.respond(response);
}

fn respond_error(request: tiny_http::Request, code: u16, message: &str) {
    let body = serde_json::json!({ "error": message });
    let json = serde_json::to_string(&body).unwrap();
    let response = Response::from_string(json)
        .with_status_code(StatusCode(code))
        .with_header("Content-Type: application/json".parse::<tiny_http::Header>().unwrap());
    let _ = request.respond(response);
}

#[derive(Serialize)]
struct CpuState {
    a: u8,
    b: u8,
    c: u8,
    d: u8,
    e: u8,
    h: u8,
    l: u8,
    sp: u16,
    pc: u16,
    flags: FlagsState,
    ime: bool,
    halted: bool,
}

#[derive(Serialize)]
struct FlagsState {
    zero: bool,
    negative: bool,
    half_carry: bool,
    carry: bool,
}

#[derive(Serialize)]
struct InstructionEntry {
    address: String,
    text: String,
}
