use std::path::PathBuf;
use std::process;

use missingno_core::debugger::Debugger;
use missingno_core::debugger::instructions::InstructionsIterator;
use missingno_core::game_boy::GameBoy;
use missingno_core::game_boy::cartridge::Cartridge;
use missingno_core::game_boy::cpu::flags::Flags;
use missingno_core::game_boy::cpu::instructions::Instruction;
use missingno_core::game_boy::interrupts;
use missingno_core::game_boy::ppu;
use missingno_core::game_boy::ppu::sprites::{Attributes, SpriteId};
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
        (&Method::Get, "/ppu") => {
            respond_json(request, ppu_state(debugger.game_boy()));
        }
        (&Method::Get, "/ppu/pipeline") => {
            respond_json(request, pipeline_state(debugger.game_boy()));
        }
        (&Method::Get, "/screen") => {
            respond_json(request, screen_state(debugger.game_boy()));
        }
        (&Method::Get, "/screen/ascii") => {
            respond_json(request, screen_ascii(debugger.game_boy()));
        }
        (&Method::Get, "/sprites") => {
            respond_json(request, sprites_state(debugger.game_boy()));
        }
        (&Method::Get, "/interrupts") => {
            respond_json(request, interrupts_state(debugger.game_boy()));
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

fn ppu_state(gb: &GameBoy) -> PpuState {
    let ppu = gb.ppu();
    let control = ppu.control();
    let mode = ppu.mode();

    PpuState {
        lcdc: LcdcState {
            raw: control.bits(),
            video_enabled: control.video_enabled(),
            window_tile_map: control.window_tile_map().0,
            window_enabled: control.window_enabled(),
            tile_address_mode: match control.tile_address_mode() {
                ppu::tiles::TileAddressMode::Block0Block1 => "8000",
                ppu::tiles::TileAddressMode::Block2Block1 => "8800",
            },
            bg_tile_map: control.background_tile_map().0,
            sprite_size: match control.sprite_size() {
                ppu::sprites::SpriteSize::Single => "8x8",
                ppu::sprites::SpriteSize::Double => "8x16",
            },
            sprites_enabled: control.sprites_enabled(),
            bg_and_window_enabled: control.background_and_window_enabled(),
        },
        stat: StatState {
            raw: ppu.read_register(ppu::Register::Status),
            mode: match mode {
                ppu::pixel_pipeline::Mode::HorizontalBlank => "hblank",
                ppu::pixel_pipeline::Mode::VerticalBlank => "vblank",
                ppu::pixel_pipeline::Mode::OamScan => "oam_scan",
                ppu::pixel_pipeline::Mode::Drawing => "drawing",
            },
            mode_number: mode as u8,
        },
        ly: ppu.read_register(ppu::Register::CurrentScanline),
        dot: ppu.dot(),
        lyc: ppu.read_register(ppu::Register::InterruptOnScanline),
        scy: ppu.read_register(ppu::Register::BackgroundViewportY),
        scx: ppu.read_register(ppu::Register::BackgroundViewportX),
        wy: ppu.read_register(ppu::Register::WindowY),
        wx: ppu.read_register(ppu::Register::WindowX),
        bgp: palette_breakdown(ppu.read_register(ppu::Register::BackgroundPalette)),
        obp0: palette_breakdown(ppu.read_register(ppu::Register::Sprite0Palette)),
        obp1: palette_breakdown(ppu.read_register(ppu::Register::Sprite1Palette)),
    }
}

fn pipeline_state(gb: &GameBoy) -> serde_json::Value {
    let ppu = gb.ppu();
    match ppu.pipeline_state() {
        Some(snap) => serde_json::json!({
            "pixel_counter": snap.pixel_counter,
            "render_phase": match snap.render_phase {
                ppu::RenderPhase::LineStart => "line_start",
                ppu::RenderPhase::OamScan => "oam_scan",
                ppu::RenderPhase::Drawing => "drawing",
                ppu::RenderPhase::DrawingComplete => "drawing_complete",
                ppu::RenderPhase::HorizontalBlank => "hblank",
            },
            "bg_shifter": {
                "low": snap.bg_low,
                "high": snap.bg_high,
                "loaded": snap.bg_loaded,
            },
            "obj_shifter": {
                "low": snap.obj_low,
                "high": snap.obj_high,
                "palette": snap.obj_palette,
                "priority": snap.obj_priority,
            },
            "sprite_fetch": match snap.sprite_fetch_phase {
                Some(ppu::SpriteFetchPhase::WaitingForFetcher) => serde_json::Value::String("waiting".into()),
                Some(ppu::SpriteFetchPhase::FetchingData) => serde_json::Value::String("fetching_data".into()),
                Some(ppu::SpriteFetchPhase::Done) => serde_json::Value::String("done".into()),
                None => serde_json::Value::Null,
            },
        }),
        None => serde_json::Value::Null,
    }
}

fn palette_breakdown(raw: u8) -> PaletteState {
    PaletteState {
        raw,
        colors: [raw & 3, (raw >> 2) & 3, (raw >> 4) & 3, (raw >> 6) & 3],
    }
}

fn screen_state(gb: &GameBoy) -> ScreenState {
    let screen = gb.screen();
    let mut lines = Vec::with_capacity(144);
    for y in 0..144u8 {
        let mut row = Vec::with_capacity(160);
        for x in 0..160u8 {
            row.push(screen.pixel(x, y).0);
        }
        lines.push(row);
    }
    ScreenState { pixels: lines }
}

fn screen_ascii(gb: &GameBoy) -> ScreenAscii {
    let screen = gb.screen();
    let shades = [' ', '.', 'o', '#'];
    let mut lines = Vec::with_capacity(144);
    for y in 0..144u8 {
        let mut row = String::with_capacity(160);
        for x in 0..160u8 {
            row.push(shades[screen.pixel(x, y).0 as usize]);
        }
        lines.push(row);
    }
    ScreenAscii { lines }
}

fn sprites_state(gb: &GameBoy) -> Vec<SpriteState> {
    let ppu = gb.ppu();
    let sprite_size = ppu.control().sprite_size();
    (0..40)
        .map(|i| {
            let sprite = ppu.sprite(SpriteId(i));
            let x = sprite.position.x_plus_8 as i16 - 8;
            let y = sprite.position.y_plus_16 as i16 - 16;
            SpriteState {
                id: i,
                x,
                y,
                tile: sprite.tile.0,
                priority: if sprite.attributes.contains(Attributes::PRIORITY) {
                    "behind_bg"
                } else {
                    "above_bg"
                },
                flip_x: sprite.attributes.contains(Attributes::FLIP_X),
                flip_y: sprite.attributes.contains(Attributes::FLIP_Y),
                palette: if sprite.attributes.contains(Attributes::PALETTE) {
                    "obp1"
                } else {
                    "obp0"
                },
                visible: sprite.position.on_screen_x() && sprite.position.on_screen_y(sprite_size),
            }
        })
        .collect()
}

fn interrupts_state(gb: &GameBoy) -> InterruptsState {
    let regs = gb.interrupts();
    let check = |flag: interrupts::Interrupt| -> InterruptLine {
        InterruptLine {
            enabled: regs.enabled(flag),
            requested: regs.requested(flag),
        }
    };
    InterruptsState {
        ie_raw: regs.enabled.bits() & 0x1F,
        if_raw: regs.requested.bits() & 0x1F,
        vblank: check(interrupts::Interrupt::VideoBetweenFrames),
        stat: check(interrupts::Interrupt::VideoStatus),
        timer: check(interrupts::Interrupt::Timer),
        serial: check(interrupts::Interrupt::Serial),
        joypad: check(interrupts::Interrupt::Joypad),
    }
}

fn respond_json(request: tiny_http::Request, body: impl Serialize) {
    let json = serde_json::to_string_pretty(&body).unwrap();
    let response = Response::from_string(json).with_header(
        "Content-Type: application/json"
            .parse::<tiny_http::Header>()
            .unwrap(),
    );
    let _ = request.respond(response);
}

fn respond_error(request: tiny_http::Request, code: u16, message: &str) {
    let body = serde_json::json!({ "error": message });
    let json = serde_json::to_string(&body).unwrap();
    let response = Response::from_string(json)
        .with_status_code(StatusCode(code))
        .with_header(
            "Content-Type: application/json"
                .parse::<tiny_http::Header>()
                .unwrap(),
        );
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

#[derive(Serialize)]
struct PpuState {
    lcdc: LcdcState,
    stat: StatState,
    ly: u8,
    dot: u32,
    lyc: u8,
    scy: u8,
    scx: u8,
    wy: u8,
    wx: u8,
    bgp: PaletteState,
    obp0: PaletteState,
    obp1: PaletteState,
}

#[derive(Serialize)]
struct LcdcState {
    raw: u8,
    video_enabled: bool,
    window_tile_map: u8,
    window_enabled: bool,
    tile_address_mode: &'static str,
    bg_tile_map: u8,
    sprite_size: &'static str,
    sprites_enabled: bool,
    bg_and_window_enabled: bool,
}

#[derive(Serialize)]
struct StatState {
    raw: u8,
    mode: &'static str,
    mode_number: u8,
}

#[derive(Serialize)]
struct PaletteState {
    raw: u8,
    colors: [u8; 4],
}

#[derive(Serialize)]
struct ScreenState {
    pixels: Vec<Vec<u8>>,
}

#[derive(Serialize)]
struct ScreenAscii {
    lines: Vec<String>,
}

#[derive(Serialize)]
struct SpriteState {
    id: u8,
    x: i16,
    y: i16,
    tile: u8,
    priority: &'static str,
    flip_x: bool,
    flip_y: bool,
    palette: &'static str,
    visible: bool,
}

#[derive(Serialize)]
struct InterruptsState {
    ie_raw: u8,
    if_raw: u8,
    vblank: InterruptLine,
    stat: InterruptLine,
    timer: InterruptLine,
    serial: InterruptLine,
    joypad: InterruptLine,
}

#[derive(Serialize)]
struct InterruptLine {
    enabled: bool,
    requested: bool,
}
