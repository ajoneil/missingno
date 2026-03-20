use std::path::PathBuf;
use std::process;

use missingno_core::debugger::instructions::InstructionsIterator;
use missingno_core::debugger::{Debugger, WatchCondition};
use missingno_core::game_boy::cartridge::Cartridge;
use missingno_core::game_boy::cpu::flags::Flags;
use missingno_core::game_boy::cpu::instructions::Instruction;
use missingno_core::game_boy::interrupts;
use missingno_core::game_boy::ppu;
use missingno_core::game_boy::ppu::pixel_pipeline::Mode;
use missingno_core::game_boy::ppu::sprites::{Attributes, SpriteId};
use missingno_core::game_boy::{ClockPhase, GameBoy};
use serde::Serialize;
use tiny_http::{Method, Response, StatusCode};

pub fn run(rom_path: Option<PathBuf>, boot_rom: Option<Box<[u8; 256]>>) {
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
    let game_boy = GameBoy::new(cartridge, boot_rom);
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

fn handle_request(mut request: tiny_http::Request, debugger: &mut Debugger) {
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
        (&Method::Get, "/screen/bitmap") => {
            let bmp = screen_bitmap(debugger.game_boy());
            let response = Response::from_data(bmp).with_header(
                "Content-Type: image/bmp"
                    .parse::<tiny_http::Header>()
                    .unwrap(),
            );
            let _ = request.respond(response);
        }
        (&Method::Get, "/tiles/bitmap") => {
            let bmp = tiles_bitmap(debugger.game_boy());
            let response = Response::from_data(bmp).with_header(
                "Content-Type: image/bmp"
                    .parse::<tiny_http::Header>()
                    .unwrap(),
            );
            let _ = request.respond(response);
        }
        (&Method::Get, "/tilemap/0/bitmap") => {
            let bmp = tilemap_bitmap(debugger.game_boy(), ppu::tile_maps::TileMapId(0));
            let response = Response::from_data(bmp).with_header(
                "Content-Type: image/bmp"
                    .parse::<tiny_http::Header>()
                    .unwrap(),
            );
            let _ = request.respond(response);
        }
        (&Method::Get, "/tilemap/1/bitmap") => {
            let bmp = tilemap_bitmap(debugger.game_boy(), ppu::tile_maps::TileMapId(1));
            let response = Response::from_data(bmp).with_header(
                "Content-Type: image/bmp"
                    .parse::<tiny_http::Header>()
                    .unwrap(),
            );
            let _ = request.respond(response);
        }
        (&Method::Get, "/sprite-store") => match debugger.game_boy().ppu().sprite_store() {
            Some(store) => {
                let entries: Vec<serde_json::Value> = store
                    .entries
                    .iter()
                    .map(|e| {
                        serde_json::json!({
                            "oam_index": e.oam_index,
                            "line_offset": e.line_offset,
                            "x": e.x,
                            "fetched": e.fetched,
                        })
                    })
                    .collect();
                respond_json(
                    request,
                    serde_json::json!({
                        "count": store.count,
                        "fetched_mask": store.fetched,
                        "entries": entries,
                    }),
                );
            }
            None => respond_json(request, serde_json::Value::Null),
        },
        (&Method::Get, "/sprites") => {
            respond_json(request, sprites_state(debugger.game_boy()));
        }
        (&Method::Get, "/timers") => {
            respond_json(request, timers_state(debugger.game_boy()));
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
        (&Method::Post, "/step-dot") => {
            debugger.step_dot();
            respond_json(request, pipeline_state(debugger.game_boy()));
        }
        (&Method::Post, "/step-phase") => {
            debugger.step_phase();
            let mut response = serde_json::to_value(pipeline_state(debugger.game_boy())).unwrap();
            response["phase"] =
                serde_json::Value::String(match debugger.game_boy().clock_phase() {
                    ClockPhase::High => "high".to_string(),
                    ClockPhase::Low => "low".to_string(),
                });
            respond_json(request, response);
        }
        (&Method::Post, "/step-frame") => {
            debugger.step_frame();
            let mut response = serde_json::to_value(cpu_state(debugger.game_boy())).unwrap();
            if let Some(hit) = debugger.last_watchpoint_hit() {
                response["watchpoint_hit"] = watchpoint_json(hit);
            }
            respond_json(request, response);
        }
        (&Method::Post, "/step-over") => {
            debugger.step_over();
            respond_json(request, cpu_state(debugger.game_boy()));
        }
        (&Method::Post, "/reset") => {
            debugger.reset();
            respond_json(request, cpu_state(debugger.game_boy()));
        }
        (&Method::Get, "/vram") => {
            respond_json(request, vram_state(debugger.game_boy()));
        }
        _ if path.starts_with("/memory/") => {
            if method != Method::Get {
                respond_error(request, 405, "method not allowed");
                return;
            }
            let rest = &path["/memory/".len()..];
            let parts: Vec<&str> = rest.splitn(2, '/').collect();
            let addr = match u16::from_str_radix(parts[0], 16) {
                Ok(a) => a,
                Err(_) => {
                    respond_error(request, 400, "invalid hex address");
                    return;
                }
            };
            let length: u16 = if parts.len() > 1 {
                match parts[1].parse() {
                    Ok(n) if n >= 1 && n <= 0x1000 => n,
                    _ => {
                        respond_error(request, 400, "invalid length (1-4096)");
                        return;
                    }
                }
            } else {
                1
            };
            let gb = debugger.game_boy();
            let bytes: Vec<u8> = (0..length).map(|i| gb.peek(addr.wrapping_add(i))).collect();
            if length == 1 {
                respond_json(
                    request,
                    serde_json::json!({
                        "address": format!("{addr:04x}"),
                        "value": bytes[0],
                        "hex": format!("{:02x}", bytes[0]),
                    }),
                );
            } else {
                let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
                respond_json(
                    request,
                    serde_json::json!({
                        "address": format!("{addr:04x}"),
                        "length": length,
                        "bytes": bytes,
                        "hex": hex,
                    }),
                );
            }
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
        (&Method::Get, "/watchpoints") => {
            let conditions: Vec<serde_json::Value> =
                debugger.watchpoints().iter().map(watchpoint_json).collect();
            respond_json(request, conditions);
        }
        (&Method::Post, "/watchpoints") => {
            let mut body = String::new();
            request.as_reader().read_to_string(&mut body).unwrap();
            match parse_watchpoint_body(&body) {
                Ok(condition) => {
                    debugger.add_watchpoint(condition.clone());
                    respond_json(
                        request,
                        serde_json::json!({ "added": watchpoint_json(&condition) }),
                    );
                }
                Err(err) => respond_error(request, 400, &err),
            }
        }
        (&Method::Delete, "/watchpoints") => {
            debugger.clear_watchpoints();
            respond_json(request, serde_json::json!({ "cleared": "all" }));
        }
        _ if path.starts_with("/watchpoints/bus-read/") => {
            let addr_str = &path["/watchpoints/bus-read/".len()..];
            match u16::from_str_radix(addr_str, 16) {
                Ok(addr) => {
                    let condition = WatchCondition::BusRead { address: addr };
                    match &method {
                        &Method::Put => {
                            debugger.add_watchpoint(condition.clone());
                            respond_json(
                                request,
                                serde_json::json!({ "added": watchpoint_json(&condition) }),
                            );
                        }
                        &Method::Delete => {
                            debugger.remove_watchpoint(&condition);
                            respond_json(
                                request,
                                serde_json::json!({ "removed": watchpoint_json(&condition) }),
                            );
                        }
                        _ => respond_error(request, 405, "method not allowed"),
                    }
                }
                Err(_) => respond_error(request, 400, "invalid hex address"),
            }
        }
        _ if path.starts_with("/watchpoints/bus-write/") => {
            let addr_str = &path["/watchpoints/bus-write/".len()..];
            match u16::from_str_radix(addr_str, 16) {
                Ok(addr) => {
                    let condition = WatchCondition::BusWrite { address: addr };
                    match &method {
                        &Method::Put => {
                            debugger.add_watchpoint(condition.clone());
                            respond_json(
                                request,
                                serde_json::json!({ "added": watchpoint_json(&condition) }),
                            );
                        }
                        &Method::Delete => {
                            debugger.remove_watchpoint(&condition);
                            respond_json(
                                request,
                                serde_json::json!({ "removed": watchpoint_json(&condition) }),
                            );
                        }
                        _ => respond_error(request, 405, "method not allowed"),
                    }
                }
                Err(_) => respond_error(request, 400, "invalid hex address"),
            }
        }
        _ if path.starts_with("/watchpoints/dma-read/") => {
            let addr_str = &path["/watchpoints/dma-read/".len()..];
            match u16::from_str_radix(addr_str, 16) {
                Ok(addr) => {
                    let condition = WatchCondition::DmaRead { address: addr };
                    match &method {
                        &Method::Put => {
                            debugger.add_watchpoint(condition.clone());
                            respond_json(
                                request,
                                serde_json::json!({ "added": watchpoint_json(&condition) }),
                            );
                        }
                        &Method::Delete => {
                            debugger.remove_watchpoint(&condition);
                            respond_json(
                                request,
                                serde_json::json!({ "removed": watchpoint_json(&condition) }),
                            );
                        }
                        _ => respond_error(request, 405, "method not allowed"),
                    }
                }
                Err(_) => respond_error(request, 400, "invalid hex address"),
            }
        }
        _ if path.starts_with("/watchpoints/dma-write/") => {
            let addr_str = &path["/watchpoints/dma-write/".len()..];
            match u16::from_str_radix(addr_str, 16) {
                Ok(addr) => {
                    let condition = WatchCondition::DmaWrite { address: addr };
                    match &method {
                        &Method::Put => {
                            debugger.add_watchpoint(condition.clone());
                            respond_json(
                                request,
                                serde_json::json!({ "added": watchpoint_json(&condition) }),
                            );
                        }
                        &Method::Delete => {
                            debugger.remove_watchpoint(&condition);
                            respond_json(
                                request,
                                serde_json::json!({ "removed": watchpoint_json(&condition) }),
                            );
                        }
                        _ => respond_error(request, 405, "method not allowed"),
                    }
                }
                Err(_) => respond_error(request, 400, "invalid hex address"),
            }
        }
        _ if path.starts_with("/watchpoints/scanline/") => {
            let val_str = &path["/watchpoints/scanline/".len()..];
            match val_str.parse::<u8>() {
                Ok(ly) => {
                    let condition = WatchCondition::Scanline(ly);
                    match &method {
                        &Method::Put => {
                            debugger.add_watchpoint(condition.clone());
                            respond_json(
                                request,
                                serde_json::json!({ "added": watchpoint_json(&condition) }),
                            );
                        }
                        &Method::Delete => {
                            debugger.remove_watchpoint(&condition);
                            respond_json(
                                request,
                                serde_json::json!({ "removed": watchpoint_json(&condition) }),
                            );
                        }
                        _ => respond_error(request, 405, "method not allowed"),
                    }
                }
                Err(_) => respond_error(request, 400, "invalid scanline number"),
            }
        }
        _ if path.starts_with("/watchpoints/pixel-counter/") => {
            let val_str = &path["/watchpoints/pixel-counter/".len()..];
            match val_str.parse::<u8>() {
                Ok(pc) => {
                    let condition = WatchCondition::PixelCounter(pc);
                    match &method {
                        &Method::Put => {
                            debugger.add_watchpoint(condition.clone());
                            respond_json(
                                request,
                                serde_json::json!({ "added": watchpoint_json(&condition) }),
                            );
                        }
                        &Method::Delete => {
                            debugger.remove_watchpoint(&condition);
                            respond_json(
                                request,
                                serde_json::json!({ "removed": watchpoint_json(&condition) }),
                            );
                        }
                        _ => respond_error(request, 405, "method not allowed"),
                    }
                }
                Err(_) => respond_error(request, 400, "invalid pixel counter value"),
            }
        }
        _ if path.starts_with("/watchpoints/ppu-mode/") => {
            let mode_str = &path["/watchpoints/ppu-mode/".len()..];
            let mode = match mode_str {
                "hblank" | "0" => Some(Mode::HorizontalBlank),
                "vblank" | "1" => Some(Mode::VerticalBlank),
                "oam_scan" | "2" => Some(Mode::OamScan),
                "drawing" | "3" => Some(Mode::Drawing),
                _ => None,
            };
            match mode {
                Some(mode) => {
                    let condition = WatchCondition::PpuMode(mode);
                    match &method {
                        &Method::Put => {
                            debugger.add_watchpoint(condition.clone());
                            respond_json(
                                request,
                                serde_json::json!({ "added": watchpoint_json(&condition) }),
                            );
                        }
                        &Method::Delete => {
                            debugger.remove_watchpoint(&condition);
                            respond_json(
                                request,
                                serde_json::json!({ "removed": watchpoint_json(&condition) }),
                            );
                        }
                        _ => respond_error(request, 405, "method not allowed"),
                    }
                }
                None => respond_error(
                    request,
                    400,
                    "invalid mode: use hblank/vblank/oam_scan/drawing or 0/1/2/3",
                ),
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
        lx: ppu.lx(),
        lyc: ppu.read_register(ppu::Register::InterruptOnScanline),
        scan_counter: ppu.scan_counter(),
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
            "xymu": snap.xymu,
            "bg_shifter": {
                "low": snap.bg_low,
                "high": snap.bg_high,
            },
            "obj_shifter": {
                "low": snap.obj_low,
                "high": snap.obj_high,
                "palette": snap.obj_palette,
                "priority": snap.obj_priority,
            },
            "sprite_fetch": match snap.sprite_fetch_phase {
                Some(ppu::SpriteFetchPhase::FetchingData) => serde_json::Value::String("fetching_data".into()),
                None => serde_json::Value::Null,
            },
            "sprite_tile_data": match snap.sprite_tile_data {
                Some((low, high)) => serde_json::json!({"low": low, "high": high}),
                None => serde_json::Value::Null,
            },
            "lcd_x": snap.lcd_x,
            "fetcher_step": format!("phase_tfetch={}", snap.phase_tfetch),
            "rydy": snap.rydy,
            "wusa": snap.wusa,
            "pova": snap.pova,
            "pygo": snap.pygo,
            "poky": snap.poky,
            "wx_triggered": snap.wx_triggered,
            "wuvu": snap.wuvu,
            "byba": snap.byba,
            "doba": snap.doba,
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

fn write_bmp(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    let row_stride = ((width * 3 + 3) & !3) as usize;
    let pixel_data_size = row_stride * height as usize;
    let file_size = 54 + pixel_data_size;

    let mut bmp = Vec::with_capacity(file_size);

    // BMP file header (14 bytes)
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&(file_size as u32).to_le_bytes());
    bmp.extend_from_slice(&0u16.to_le_bytes());
    bmp.extend_from_slice(&0u16.to_le_bytes());
    bmp.extend_from_slice(&54u32.to_le_bytes());

    // DIB header (40 bytes)
    bmp.extend_from_slice(&40u32.to_le_bytes());
    bmp.extend_from_slice(&width.to_le_bytes());
    bmp.extend_from_slice(&(height as i32).to_le_bytes());
    bmp.extend_from_slice(&1u16.to_le_bytes());
    bmp.extend_from_slice(&24u16.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&(pixel_data_size as u32).to_le_bytes());
    bmp.extend_from_slice(&2835u32.to_le_bytes());
    bmp.extend_from_slice(&2835u32.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());

    // Pixel data (bottom-up)
    let padding = row_stride - width as usize * 3;
    for y in (0..height).rev() {
        let row_start = (y * width) as usize * 3;
        bmp.extend_from_slice(&pixels[row_start..row_start + width as usize * 3]);
        for _ in 0..padding {
            bmp.push(0);
        }
    }

    bmp
}

fn screen_bitmap(gb: &GameBoy) -> Vec<u8> {
    let screen = gb.screen();
    let greys: [u8; 4] = [0xFF, 0xAA, 0x55, 0x00];

    let mut pixels = Vec::with_capacity(160 * 144 * 3);
    for y in 0..144u8 {
        for x in 0..160u8 {
            let shade = greys[screen.pixel(x, y).0 as usize];
            pixels.push(shade);
            pixels.push(shade);
            pixels.push(shade);
        }
    }

    write_bmp(160, 144, &pixels)
}

/// Renders all 384 tiles (3 blocks of 128) in a 16-wide grid.
fn tiles_bitmap(gb: &GameBoy) -> Vec<u8> {
    let vram = gb.vram();
    let greys: [u8; 4] = [0xFF, 0xAA, 0x55, 0x00];

    // 16 tiles wide, 24 tiles tall (384 tiles total)
    let cols = 16u32;
    let rows = 24u32;
    let w = cols * 8;
    let h = rows * 8;

    let mut pixels = vec![0u8; (w * h * 3) as usize];

    for block_id in 0..3u8 {
        let block = vram.tile_block(ppu::tiles::TileBlockId(block_id));
        for tile_idx in 0..128u8 {
            let tile = block.tile(ppu::tiles::TileIndex(tile_idx));
            let global_idx = block_id as u32 * 128 + tile_idx as u32;
            let grid_x = global_idx % cols;
            let grid_y = global_idx / cols;
            for ty in 0..8u8 {
                for tx in 0..8u8 {
                    let shade = greys[tile.pixel(tx, ty).0 as usize];
                    let px = (grid_x * 8 + tx as u32) as usize;
                    let py = (grid_y * 8 + ty as u32) as usize;
                    let offset = (py * w as usize + px) * 3;
                    pixels[offset] = shade;
                    pixels[offset + 1] = shade;
                    pixels[offset + 2] = shade;
                }
            }
        }
    }

    write_bmp(w, h, &pixels)
}

/// Renders a 32x32 tile map as a 256x256 bitmap.
fn tilemap_bitmap(gb: &GameBoy, map_id: ppu::tile_maps::TileMapId) -> Vec<u8> {
    let vram = gb.vram();
    let tile_map = vram.tile_map(map_id);
    let addr_mode = gb.ppu().control().tile_address_mode();
    let greys: [u8; 4] = [0xFF, 0xAA, 0x55, 0x00];

    let w = 256u32;
    let h = 256u32;

    let mut pixels = vec![0u8; (w * h * 3) as usize];

    for map_y in 0..32u8 {
        for map_x in 0..32u8 {
            let tile_index = tile_map.get_tile(map_x, map_y);
            let (block_id, block_index) = addr_mode.tile(tile_index);
            let block = vram.tile_block(block_id);
            let tile = block.tile(block_index);
            for ty in 0..8u8 {
                for tx in 0..8u8 {
                    let shade = greys[tile.pixel(tx, ty).0 as usize];
                    let px = (map_x as u32 * 8 + tx as u32) as usize;
                    let py = (map_y as u32 * 8 + ty as u32) as usize;
                    let offset = (py * w as usize + px) * 3;
                    pixels[offset] = shade;
                    pixels[offset + 1] = shade;
                    pixels[offset + 2] = shade;
                }
            }
        }
    }

    write_bmp(w, h, &pixels)
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

fn vram_state(gb: &GameBoy) -> serde_json::Value {
    let vram = gb.vram();
    let mut tile_blocks = Vec::with_capacity(3);
    for block_id in 0..3u8 {
        let block = vram.tile_block(ppu::tiles::TileBlockId(block_id));
        let base_addr = 0x8000u16 + block_id as u16 * 0x800;
        let mut tiles = Vec::with_capacity(128);
        for tile_idx in 0..128u8 {
            let tile = block.tile(ppu::tiles::TileIndex(tile_idx));
            let offset = tile_idx as usize * 16;
            let raw: Vec<u8> = block.data[offset..offset + 16].to_vec();
            let hex: Vec<String> = raw.iter().map(|b| format!("{b:02x}")).collect();
            // Decode 8x8 pixel grid
            let mut pixels = Vec::with_capacity(8);
            for y in 0..8u8 {
                let mut row = Vec::with_capacity(8);
                for x in 0..8u8 {
                    row.push(tile.pixel(x, y).0);
                }
                pixels.push(row);
            }
            let non_zero = raw.iter().any(|&b| b != 0);
            tiles.push(serde_json::json!({
                "index": tile_idx,
                "address": format!("{:04x}", base_addr + offset as u16),
                "raw": hex,
                "pixels": pixels,
                "non_zero": non_zero,
            }));
        }
        tile_blocks.push(serde_json::json!({
            "block": block_id,
            "address": format!("{base_addr:04x}"),
            "tiles": tiles,
        }));
    }

    let mut maps = Vec::with_capacity(2);
    for map_id in 0..2u8 {
        let tile_map = vram.tile_map(ppu::tile_maps::TileMapId(map_id));
        let base_addr = 0x9800u16 + map_id as u16 * 0x400;
        let mut rows = Vec::with_capacity(32);
        for y in 0..32u8 {
            let row: Vec<u8> = (0..32u8).map(|x| tile_map.get_tile(x, y).0).collect();
            rows.push(row);
        }
        maps.push(serde_json::json!({
            "map": map_id,
            "address": format!("{base_addr:04x}"),
            "rows": rows,
        }));
    }

    serde_json::json!({
        "tile_blocks": tile_blocks,
        "tile_maps": maps,
    })
}

fn timers_state(gb: &GameBoy) -> TimersState {
    let timers = gb.timers();
    let div = timers.read_register(missingno_core::game_boy::timers::Register::Divider);
    let tima = timers.read_register(missingno_core::game_boy::timers::Register::Counter);
    let tma = timers.read_register(missingno_core::game_boy::timers::Register::Modulo);
    let tac = timers.read_register(missingno_core::game_boy::timers::Register::Control);
    let internal = timers.internal_counter();
    let clock_select = tac & 0b11;
    let freq = match clock_select {
        0b00 => 4096,
        0b01 => 262144,
        0b10 => 65536,
        0b11.. => 16384,
    };
    TimersState {
        div,
        tima,
        tma,
        tac,
        timer_enabled: tac & 0b100 != 0,
        clock_select,
        frequency: freq,
        internal_counter: format!("{internal:04x}"),
        internal_counter_decimal: internal,
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

fn watchpoint_json(condition: &WatchCondition) -> serde_json::Value {
    match condition {
        WatchCondition::BusRead { address } => serde_json::json!({
            "type": "bus_read",
            "address": format!("{address:04x}"),
        }),
        WatchCondition::BusWrite { address } => serde_json::json!({
            "type": "bus_write",
            "address": format!("{address:04x}"),
        }),
        WatchCondition::DmaRead { address } => serde_json::json!({
            "type": "dma_read",
            "address": format!("{address:04x}"),
        }),
        WatchCondition::DmaWrite { address } => serde_json::json!({
            "type": "dma_write",
            "address": format!("{address:04x}"),
        }),
        WatchCondition::Scanline(ly) => serde_json::json!({
            "type": "scanline",
            "value": ly,
        }),
        WatchCondition::PpuMode(mode) => serde_json::json!({
            "type": "ppu_mode",
            "mode": match mode {
                Mode::HorizontalBlank => "hblank",
                Mode::VerticalBlank => "vblank",
                Mode::OamScan => "oam_scan",
                Mode::Drawing => "drawing",
            },
        }),
        WatchCondition::PixelCounter(pc) => serde_json::json!({
            "type": "pixel_counter",
            "value": pc,
        }),
        WatchCondition::PpuRegister { register, value } => serde_json::json!({
            "type": "ppu_register",
            "register": format!("{register:?}"),
            "value": value,
        }),
        WatchCondition::CpuRegister { register, value } => serde_json::json!({
            "type": "cpu_register",
            "register": format!("{register:?}"),
            "value": value,
        }),
        WatchCondition::All(conditions) => serde_json::json!({
            "type": "all",
            "conditions": conditions.iter().map(watchpoint_json).collect::<Vec<_>>(),
        }),
    }
}

fn parse_watchpoint_body(body: &str) -> Result<WatchCondition, String> {
    let json: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("invalid JSON: {e}"))?;
    parse_watchpoint_json(&json)
}

fn parse_watchpoint_json(json: &serde_json::Value) -> Result<WatchCondition, String> {
    let typ = json["type"].as_str().ok_or("missing \"type\" field")?;
    match typ {
        "bus_read" => {
            let addr = parse_hex_field(json, "address")?;
            Ok(WatchCondition::BusRead { address: addr })
        }
        "bus_write" => {
            let addr = parse_hex_field(json, "address")?;
            Ok(WatchCondition::BusWrite { address: addr })
        }
        "dma_read" => {
            let addr = parse_hex_field(json, "address")?;
            Ok(WatchCondition::DmaRead { address: addr })
        }
        "dma_write" => {
            let addr = parse_hex_field(json, "address")?;
            Ok(WatchCondition::DmaWrite { address: addr })
        }
        "scanline" => {
            let value = json["value"].as_u64().ok_or("missing \"value\" field")? as u8;
            Ok(WatchCondition::Scanline(value))
        }
        "pixel_counter" => {
            let value = json["value"].as_u64().ok_or("missing \"value\" field")? as u8;
            Ok(WatchCondition::PixelCounter(value))
        }
        "ppu_mode" => {
            let mode_str = json["mode"].as_str().ok_or("missing \"mode\" field")?;
            let mode = match mode_str {
                "hblank" | "0" => Mode::HorizontalBlank,
                "vblank" | "1" => Mode::VerticalBlank,
                "oam_scan" | "2" => Mode::OamScan,
                "drawing" | "3" => Mode::Drawing,
                _ => return Err(format!("invalid mode: {mode_str}")),
            };
            Ok(WatchCondition::PpuMode(mode))
        }
        "all" => {
            let conditions = json["conditions"]
                .as_array()
                .ok_or("missing \"conditions\" array")?;
            let parsed: Result<Vec<_>, _> = conditions.iter().map(parse_watchpoint_json).collect();
            Ok(WatchCondition::All(parsed?))
        }
        other => Err(format!("unknown type: {other}")),
    }
}

fn parse_hex_field(json: &serde_json::Value, field: &str) -> Result<u16, String> {
    let s = json[field]
        .as_str()
        .ok_or(format!("missing \"{field}\" field"))?;
    u16::from_str_radix(s, 16).map_err(|_| format!("invalid hex in \"{field}\": {s}"))
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
    lx: u8,
    lyc: u8,
    scan_counter: Option<u8>,
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

#[derive(Serialize)]
struct TimersState {
    div: u8,
    tima: u8,
    tma: u8,
    tac: u8,
    timer_enabled: bool,
    clock_select: u8,
    frequency: u32,
    internal_counter: String,
    internal_counter_decimal: u16,
}
