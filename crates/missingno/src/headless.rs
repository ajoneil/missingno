use std::path::PathBuf;
use std::process;

use missingno_gb::debugger::{Debugger, WatchCondition};
use missingno_gb::ppu;
use missingno_gb::ppu::rendering::Mode;
use missingno_gb::ppu::types::palette::Palette;
use missingno_gb::{BootRom, Console, Dmg, GameBoy, Model};
use missingno_gbc::{Cgb, GameBoyColor};

use crate::render;
use serde::Serialize;
use tiny_http::{Method, Response, StatusCode};

mod bitmap;
mod inspect;
mod watchpoints;

use bitmap::{respond_bmp, rgba_to_rgb, tiles_bitmap, write_bmp};
use inspect::{
    audio_state, cpu_state, disassemble, interrupts_state, pipeline_state, ppu_state,
    sprites_state, timers_state, trace_apu, vram_state,
};
use watchpoints::{parse_watchpoint_body, watchpoint_json};

pub fn run(
    rom_path: Option<PathBuf>,
    boot_rom: Option<BootRom>,
    link: Option<Box<dyn missingno_gb::serial_transfer::SerialLink>>,
) {
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

    struct Serve;
    impl crate::app::system::gb::GbLaunch for Serve {
        type Output = ();
        fn dmg(self, console: GameBoy) {
            serve_console(console);
        }
        fn cgb(self, console: GameBoyColor) {
            serve_console(console);
        }
    }
    crate::app::system::gb::launch(rom_data, save_data, boot_rom, link, Serve);
}

fn serve_console<M: HeadlessUi>(console: Console<M>) {
    let title = console.cartridge().title().to_string();
    serve(&title, Debugger::new(console));
}

fn serve<M: HeadlessUi>(title: &str, mut debugger: Debugger<M>) {
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

/// The model-specific views the HTTP endpoints expose: the screen's pixel
/// format, colour sources for the tile-map render, and CGB palette RAM.
trait HeadlessUi: Model {
    /// What `screen_values` pixels hold, reported in the /screen JSON.
    const PIXEL_FORMAT: &'static str;
    /// Raw per-pixel values: 2-bit shades on DMG, RGB555 on CGB.
    fn screen_values(console: &Console<Self>) -> Vec<Vec<u16>>;
    /// 160×144 RGB888 of the displayed frame.
    fn screen_rgb(console: &Console<Self>) -> Vec<u8>;
    /// 256×256 RGB888 of a tile map, colour-resolved per model.
    fn tilemap_rgb(console: &Console<Self>, map_id: ppu::types::tiles::TileMapId) -> Vec<u8>;
    /// CGB palette RAM; null on DMG.
    fn cram_json(console: &Console<Self>) -> serde_json::Value;
}

impl HeadlessUi for Dmg {
    const PIXEL_FORMAT: &'static str = "shade2";

    fn screen_values(console: &Console<Self>) -> Vec<Vec<u16>> {
        let screen = console.screen();
        (0..144u8)
            .map(|y| (0..160u8).map(|x| screen.pixel(x, y).0 as u16).collect())
            .collect()
    }

    fn screen_rgb(console: &Console<Self>) -> Vec<u8> {
        let screen = console.screen();
        let greys: [u8; 4] = [0xFF, 0xAA, 0x55, 0x00];
        let mut pixels = Vec::with_capacity(160 * 144 * 3);
        for y in 0..144u8 {
            for x in 0..160u8 {
                let shade = greys[screen.pixel(x, y).0 as usize];
                pixels.extend_from_slice(&[shade, shade, shade]);
            }
        }
        pixels
    }

    fn tilemap_rgb(console: &Console<Self>, map_id: ppu::types::tiles::TileMapId) -> Vec<u8> {
        rgba_to_rgb(&render::tile_map_rgba(
            console.vram(),
            map_id,
            console.ppu().control(),
            &Palette::CLASSIC,
        ))
    }

    fn cram_json(_console: &Console<Self>) -> serde_json::Value {
        serde_json::Value::Null
    }
}

impl HeadlessUi for Cgb {
    const PIXEL_FORMAT: &'static str = "rgb555";

    fn screen_values(console: &Console<Self>) -> Vec<Vec<u16>> {
        let screen = console.screen();
        (0..144u8)
            .map(|y| (0..160u8).map(|x| screen.pixel(x, y).0).collect())
            .collect()
    }

    fn screen_rgb(console: &Console<Self>) -> Vec<u8> {
        let screen = console.screen();
        let mut pixels = Vec::with_capacity(160 * 144 * 3);
        for y in 0..144u8 {
            for x in 0..160u8 {
                let c = screen.pixel(x, y).to_corrected_rgb8();
                pixels.extend_from_slice(&[c.r, c.g, c.b]);
            }
        }
        pixels
    }

    fn tilemap_rgb(console: &Console<Self>, map_id: ppu::types::tiles::TileMapId) -> Vec<u8> {
        let cgb_ppu = console.ppu().model();
        let bg_palettes = render::cram_palettes(|palette, index| cgb_ppu.bg_color(palette, index));
        rgba_to_rgb(&render::tile_map_rgba_cgb(
            console.vram(),
            map_id,
            console.ppu().control(),
            &bg_palettes,
        ))
    }

    fn cram_json(console: &Console<Self>) -> serde_json::Value {
        let cgb_ppu = console.ppu().model();
        let palettes = |color: &dyn Fn(u8, u8) -> missingno_gbc::screen::Color555| {
            (0..8u8)
                .map(|palette| {
                    (0..4u8)
                        .map(|index| {
                            let raw = color(palette, index);
                            let rgb = raw.to_corrected_rgb8();
                            serde_json::json!({
                                "rgb555": format!("{:04x}", raw.0),
                                "corrected": format!("#{:02x}{:02x}{:02x}", rgb.r, rgb.g, rgb.b),
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        };
        serde_json::json!({
            "background": palettes(&|p, i| cgb_ppu.bg_color(p, i)),
            "objects": palettes(&|p, i| cgb_ppu.obj_color(p, i)),
        })
    }
}

fn handle_request<M: HeadlessUi>(mut request: tiny_http::Request, debugger: &mut Debugger<M>) {
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
            respond_json(
                request,
                serde_json::json!({
                    "format": M::PIXEL_FORMAT,
                    "pixels": M::screen_values(debugger.game_boy()),
                }),
            );
        }
        (&Method::Get, "/screen/ascii") => {
            let shades = [' ', '.', 'o', '#'];
            let rgb = M::screen_rgb(debugger.game_boy());
            let lines: Vec<String> = rgb
                .chunks_exact(160 * 3)
                .map(|row| {
                    row.chunks_exact(3)
                        .map(|c| {
                            let luma =
                                (c[0] as u32 * 299 + c[1] as u32 * 587 + c[2] as u32 * 114) / 1000;
                            shades[3 - (luma / 64).min(3) as usize]
                        })
                        .collect()
                })
                .collect();
            respond_json(request, ScreenAscii { lines });
        }
        (&Method::Get, "/screen/bitmap") => {
            let bmp = write_bmp(160, 144, &M::screen_rgb(debugger.game_boy()));
            respond_bmp(request, bmp);
        }
        (&Method::Get, "/tiles/bitmap") => {
            respond_bmp(request, tiles_bitmap(debugger.game_boy(), 0));
        }
        (&Method::Get, "/tiles/bitmap/1") => {
            respond_bmp(request, tiles_bitmap(debugger.game_boy(), 1));
        }
        (&Method::Get, "/tilemap/0/bitmap") => {
            let pixels = M::tilemap_rgb(debugger.game_boy(), ppu::types::tiles::TileMapId(0));
            respond_bmp(request, write_bmp(256, 256, &pixels));
        }
        (&Method::Get, "/tilemap/1/bitmap") => {
            let pixels = M::tilemap_rgb(debugger.game_boy(), ppu::types::tiles::TileMapId(1));
            respond_bmp(request, write_bmp(256, 256, &pixels));
        }
        (&Method::Get, "/cram") => {
            respond_json(request, M::cram_json(debugger.game_boy()));
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
        (&Method::Get, "/audio") => {
            respond_json(request, audio_state(debugger.game_boy()));
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
            debugger.step_tcycle();
            respond_json(request, pipeline_state(debugger.game_boy()));
        }
        (&Method::Post, "/step-phase") => {
            // Half-phase stepping was removed; the finest exposed unit is the
            // T-cycle (one dot at single speed). Retained for API stability.
            debugger.step_tcycle();
            respond_json(request, pipeline_state(debugger.game_boy()));
        }
        (&Method::Post, path) if path.starts_with("/trace-apu/") => {
            let n: usize = path.trim_start_matches("/trace-apu/").parse().unwrap_or(0);
            let trace = trace_apu(debugger, n);
            respond_json(request, trace);
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
            respond_json(request, vram_state(debugger.game_boy(), 0));
        }
        (&Method::Get, "/vram/0") => {
            respond_json(request, vram_state(debugger.game_boy(), 0));
        }
        (&Method::Get, "/vram/1") => {
            respond_json(request, vram_state(debugger.game_boy(), 1));
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
                    Ok(n) if (1..=0x1000).contains(&n) => n,
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
                Ok(addr) => match method {
                    Method::Put => {
                        debugger.set_breakpoint(addr);
                        respond_json(request, serde_json::json!({ "set": format!("{addr:04x}") }));
                    }
                    Method::Delete => {
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
                    match method {
                        Method::Put => {
                            debugger.add_watchpoint(condition.clone());
                            respond_json(
                                request,
                                serde_json::json!({ "added": watchpoint_json(&condition) }),
                            );
                        }
                        Method::Delete => {
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
                    match method {
                        Method::Put => {
                            debugger.add_watchpoint(condition.clone());
                            respond_json(
                                request,
                                serde_json::json!({ "added": watchpoint_json(&condition) }),
                            );
                        }
                        Method::Delete => {
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
                    match method {
                        Method::Put => {
                            debugger.add_watchpoint(condition.clone());
                            respond_json(
                                request,
                                serde_json::json!({ "added": watchpoint_json(&condition) }),
                            );
                        }
                        Method::Delete => {
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
                    match method {
                        Method::Put => {
                            debugger.add_watchpoint(condition.clone());
                            respond_json(
                                request,
                                serde_json::json!({ "added": watchpoint_json(&condition) }),
                            );
                        }
                        Method::Delete => {
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
                    match method {
                        Method::Put => {
                            debugger.add_watchpoint(condition.clone());
                            respond_json(
                                request,
                                serde_json::json!({ "added": watchpoint_json(&condition) }),
                            );
                        }
                        Method::Delete => {
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
                    match method {
                        Method::Put => {
                            debugger.add_watchpoint(condition.clone());
                            respond_json(
                                request,
                                serde_json::json!({ "added": watchpoint_json(&condition) }),
                            );
                        }
                        Method::Delete => {
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
                    match method {
                        Method::Put => {
                            debugger.add_watchpoint(condition.clone());
                            respond_json(
                                request,
                                serde_json::json!({ "added": watchpoint_json(&condition) }),
                            );
                        }
                        Method::Delete => {
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
struct ScreenAscii {
    lines: Vec<String>,
}
