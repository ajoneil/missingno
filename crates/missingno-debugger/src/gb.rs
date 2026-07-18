//! The Game Boy family's headless extension: the model-specific routes the
//! generic seam cannot express, at exact wire parity with the frontend's
//! original headless server. It reaches the concrete debugger through the
//! session's [`as_any_mut`] downcast — trying the DMG then the CGB model — and
//! declines any route it does not own so the generic routes still serve.
//!
//! [`as_any_mut`]: missingno_core::system::SystemDebugger::as_any_mut

use missingno_gb::cpu::flags::Flags;
use missingno_gb::cpu::instructions::Instruction;
use missingno_gb::debugger::WatchCondition;
use missingno_gb::debugger::instructions::InstructionsIterator;
use missingno_gb::ppu::memory::Vram;
use missingno_gb::ppu::rendering::Mode;
use missingno_gb::ppu::types::palette::Palette;
use missingno_gb::ppu::types::sprites::{Attributes, SpriteId};
use missingno_gb::system::{ConsoleUi, GbDebugger};
use missingno_gb::{Console, Dmg, Model, interrupts, ppu};
use missingno_gbc::Cgb;
use missingno_gbc::screen::Color555;

use serde::Serialize;
use serde_json::Value;
use tiny_http::{Header, Method, Request, Response, StatusCode};

use crate::http::Dispatch;
use crate::session::Session;

/// The model-specific views the HTTP endpoints expose: the screen's pixel
/// format, colour sources for the tile-map render, and CGB palette RAM.
trait HeadlessUi: ConsoleUi {
    /// What `screen_values` pixels hold, reported in the /screen JSON.
    const PIXEL_FORMAT: &'static str;
    /// Raw per-pixel values: 2-bit shades on DMG, RGB555 on CGB.
    fn screen_values(console: &Console<Self>) -> Vec<Vec<u16>>;
    /// 160×144 RGB888 of the displayed frame.
    fn screen_rgb(console: &Console<Self>) -> Vec<u8>;
    /// 256×256 RGB888 of a tile map, colour-resolved per model.
    fn tilemap_rgb(console: &Console<Self>, map_id: ppu::types::tiles::TileMapId) -> Vec<u8>;
    /// CGB palette RAM; null on DMG.
    fn cram_json(console: &Console<Self>) -> Value;
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
        rgba_to_rgb(&missingno_gb::render::tile_map_rgba(
            console.vram(),
            map_id,
            console.ppu().control(),
            &Palette::CLASSIC,
        ))
    }

    fn cram_json(_console: &Console<Self>) -> Value {
        Value::Null
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
        let bg_palettes =
            missingno_gbc::cram_palettes(|palette, index| cgb_ppu.bg_color(palette, index));
        rgba_to_rgb(&missingno_gbc::render::tile_map_rgba_cgb(
            console.vram(),
            map_id,
            console.ppu().control(),
            &bg_palettes,
        ))
    }

    fn cram_json(console: &Console<Self>) -> Value {
        let cgb_ppu = console.ppu().model();
        let palettes = |color: &dyn Fn(u8, u8) -> Color555| {
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

/// Which Game Boy model this session runs, resolved once per request.
enum GbModel {
    Dmg,
    Cgb,
}

fn dmg(session: &mut Session) -> &mut GbDebugger<Dmg> {
    session
        .debugger_mut()
        .as_any_mut()
        .downcast_mut::<GbDebugger<Dmg>>()
        .expect("session resolved to the DMG model")
}

fn cgb(session: &mut Session) -> &mut GbDebugger<Cgb> {
    session
        .debugger_mut()
        .as_any_mut()
        .downcast_mut::<GbDebugger<Cgb>>()
        .expect("session resolved to the CGB model")
}

/// The Game Boy extension entry point.
pub fn extension(session: &mut Session, request: Request, method: &Method, path: &str) -> Dispatch {
    let model = {
        let any = session.debugger_mut().as_any_mut();
        if any.is::<GbDebugger<Dmg>>() {
            GbModel::Dmg
        } else if any.is::<GbDebugger<Cgb>>() {
            GbModel::Cgb
        } else {
            return Dispatch::Declined(request);
        }
    };

    // Coarse stepping and breakpoints run through the session so its run
    // bookkeeping stays authoritative; the response bodies read back through
    // the concrete debugger. The paths that overlap the generic routes keep
    // their original shapes here.
    match (method, path) {
        (Method::Post, "/step") => {
            session.step();
            let state = cpu_state_owned(session, &model);
            respond_json(request, state);
            return Dispatch::Handled;
        }
        (Method::Post, "/step-over") => {
            session.step_over();
            let state = cpu_state_owned(session, &model);
            respond_json(request, state);
            return Dispatch::Handled;
        }
        (Method::Post, "/reset") => {
            session.reset();
            let state = cpu_state_owned(session, &model);
            respond_json(request, state);
            return Dispatch::Handled;
        }
        (Method::Post, "/step-frame") => {
            session.step_frame();
            let mut response = serde_json::to_value(cpu_state_owned(session, &model)).unwrap();
            let hit = match model {
                GbModel::Dmg => dmg(session).last_watchpoint_hit().map(watchpoint_json),
                GbModel::Cgb => cgb(session).last_watchpoint_hit().map(watchpoint_json),
            };
            if let Some(hit) = hit {
                response["watchpoint_hit"] = hit;
            }
            respond_json(request, response);
            return Dispatch::Handled;
        }
        (Method::Get, "/breakpoints") => {
            let addrs: Vec<String> = session
                .breakpoints()
                .iter()
                .map(|a| format!("{a:04x}"))
                .collect();
            respond_json(request, addrs);
            return Dispatch::Handled;
        }
        (_, p) if p.starts_with("/breakpoints/") => {
            let addr_str = &p["/breakpoints/".len()..];
            match u16::from_str_radix(addr_str, 16) {
                Ok(addr) => match method {
                    Method::Put => {
                        session.set_breakpoint(addr as u32);
                        respond_json(request, serde_json::json!({ "set": format!("{addr:04x}") }));
                    }
                    Method::Delete => {
                        session.clear_breakpoint(addr as u32);
                        respond_json(
                            request,
                            serde_json::json!({ "cleared": format!("{addr:04x}") }),
                        );
                    }
                    _ => respond_error(request, 405, "method not allowed"),
                },
                Err(_) => respond_error(request, 400, "invalid hex address"),
            }
            return Dispatch::Handled;
        }
        _ => {}
    }

    // Everything else runs against the model-resolved concrete debugger.
    match model {
        GbModel::Dmg => serve_gb(dmg(session), request, method, path),
        GbModel::Cgb => serve_gb(cgb(session), request, method, path),
    }
}

/// A CPU-state struct read back after a session-driven step.
fn cpu_state_owned(session: &mut Session, model: &GbModel) -> CpuState {
    match model {
        GbModel::Dmg => cpu_state(dmg(session).console()),
        GbModel::Cgb => cpu_state(cgb(session).console()),
    }
}

/// The Game Boy read, dot-step, and watchpoint routes, generic over the model.
fn serve_gb<M: HeadlessUi>(
    gbdbg: &mut GbDebugger<M>,
    request: Request,
    method: &Method,
    path: &str,
) -> Dispatch {
    match (method, path) {
        (Method::Get, "/cpu") => respond_json(request, cpu_state(gbdbg.console())),
        (Method::Get, "/ppu") => respond_json(request, ppu_state(gbdbg.console())),
        (Method::Get, "/ppu/pipeline") => respond_json(request, pipeline_state(gbdbg.console())),
        (Method::Get, "/screen") => respond_json(
            request,
            serde_json::json!({
                "format": M::PIXEL_FORMAT,
                "pixels": M::screen_values(gbdbg.console()),
            }),
        ),
        (Method::Get, "/screen/ascii") => {
            let shades = [' ', '.', 'o', '#'];
            let rgb = M::screen_rgb(gbdbg.console());
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
        (Method::Get, "/screen/bitmap") => {
            let bmp = write_bmp(160, 144, &M::screen_rgb(gbdbg.console()));
            respond_bmp(request, bmp);
        }
        (Method::Get, "/tiles/bitmap") => respond_bmp(request, tiles_bitmap(gbdbg.console(), 0)),
        (Method::Get, "/tiles/bitmap/1") => respond_bmp(request, tiles_bitmap(gbdbg.console(), 1)),
        (Method::Get, "/tilemap/0/bitmap") => {
            let pixels = M::tilemap_rgb(gbdbg.console(), ppu::types::tiles::TileMapId(0));
            respond_bmp(request, write_bmp(256, 256, &pixels));
        }
        (Method::Get, "/tilemap/1/bitmap") => {
            let pixels = M::tilemap_rgb(gbdbg.console(), ppu::types::tiles::TileMapId(1));
            respond_bmp(request, write_bmp(256, 256, &pixels));
        }
        (Method::Get, "/cram") => respond_json(request, M::cram_json(gbdbg.console())),
        (Method::Get, "/sprite-store") => match gbdbg.console().ppu().sprite_store() {
            Some(store) => {
                let entries: Vec<Value> = store
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
            None => respond_json(request, Value::Null),
        },
        (Method::Get, "/sprites") => respond_json(request, sprites_state(gbdbg.console())),
        (Method::Get, "/timers") => respond_json(request, timers_state(gbdbg.console())),
        (Method::Get, "/audio") => respond_json(request, audio_state(gbdbg.console())),
        (Method::Get, "/interrupts") => respond_json(request, interrupts_state(gbdbg.console())),
        (Method::Get, "/instructions") => respond_json(request, disassemble(gbdbg.console(), 20)),
        (Method::Post, "/step-dot") => {
            gbdbg.step_tcycle();
            respond_json(request, pipeline_state(gbdbg.console()));
        }
        (Method::Post, "/step-phase") => {
            // The finest exposed unit is the T-cycle (one dot at single
            // speed); retained for API stability.
            gbdbg.step_tcycle();
            respond_json(request, pipeline_state(gbdbg.console()));
        }
        (Method::Post, p) if p.starts_with("/trace-apu/") => {
            let n: usize = p.trim_start_matches("/trace-apu/").parse().unwrap_or(0);
            respond_json(request, trace_apu(gbdbg, n));
        }
        (Method::Get, "/vram") => respond_json(request, vram_state(gbdbg.console(), 0)),
        (Method::Get, "/vram/0") => respond_json(request, vram_state(gbdbg.console(), 0)),
        (Method::Get, "/vram/1") => respond_json(request, vram_state(gbdbg.console(), 1)),
        (_, p) if p.starts_with("/memory/") => {
            if *method != Method::Get {
                respond_error(request, 405, "method not allowed");
                return Dispatch::Handled;
            }
            let rest = &p["/memory/".len()..];
            let parts: Vec<&str> = rest.splitn(2, '/').collect();
            let addr = match u16::from_str_radix(parts[0], 16) {
                Ok(a) => a,
                Err(_) => {
                    respond_error(request, 400, "invalid hex address");
                    return Dispatch::Handled;
                }
            };
            let length: u16 = if parts.len() > 1 {
                match parts[1].parse() {
                    Ok(n) if (1..=0x1000).contains(&n) => n,
                    _ => {
                        respond_error(request, 400, "invalid length (1-4096)");
                        return Dispatch::Handled;
                    }
                }
            } else {
                1
            };
            let console = gbdbg.console();
            let bytes: Vec<u8> = (0..length)
                .map(|i| console.peek(addr.wrapping_add(i)))
                .collect();
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
        (Method::Get, "/watchpoints") => {
            let conditions: Vec<Value> = gbdbg.watchpoints().iter().map(watchpoint_json).collect();
            respond_json(request, conditions);
        }
        (Method::Post, "/watchpoints") => {
            let mut body = String::new();
            let mut request = request;
            request.as_reader().read_to_string(&mut body).unwrap();
            match parse_watchpoint_body(&body) {
                Ok(condition) => {
                    gbdbg.add_watchpoint(condition.clone());
                    respond_json(
                        request,
                        serde_json::json!({ "added": watchpoint_json(&condition) }),
                    );
                }
                Err(err) => respond_error(request, 400, &err),
            }
        }
        (Method::Delete, "/watchpoints") => {
            gbdbg.clear_watchpoints();
            respond_json(request, serde_json::json!({ "cleared": "all" }));
        }
        (_, p) if p.starts_with("/watchpoints/bus-read/") => {
            watchpoint_address(
                gbdbg,
                request,
                method,
                &p["/watchpoints/bus-read/".len()..],
                |a| WatchCondition::BusRead { address: a },
            );
        }
        (_, p) if p.starts_with("/watchpoints/bus-write/") => {
            watchpoint_address(
                gbdbg,
                request,
                method,
                &p["/watchpoints/bus-write/".len()..],
                |a| WatchCondition::BusWrite { address: a },
            );
        }
        (_, p) if p.starts_with("/watchpoints/dma-read/") => {
            watchpoint_address(
                gbdbg,
                request,
                method,
                &p["/watchpoints/dma-read/".len()..],
                |a| WatchCondition::DmaRead { address: a },
            );
        }
        (_, p) if p.starts_with("/watchpoints/dma-write/") => {
            watchpoint_address(
                gbdbg,
                request,
                method,
                &p["/watchpoints/dma-write/".len()..],
                |a| WatchCondition::DmaWrite { address: a },
            );
        }
        (_, p) if p.starts_with("/watchpoints/scanline/") => {
            let val_str = &p["/watchpoints/scanline/".len()..];
            match val_str.parse::<u8>() {
                Ok(ly) => watchpoint_edit(gbdbg, request, method, WatchCondition::Scanline(ly)),
                Err(_) => respond_error(request, 400, "invalid scanline number"),
            }
        }
        (_, p) if p.starts_with("/watchpoints/pixel-counter/") => {
            let val_str = &p["/watchpoints/pixel-counter/".len()..];
            match val_str.parse::<u8>() {
                Ok(pc) => watchpoint_edit(gbdbg, request, method, WatchCondition::PixelCounter(pc)),
                Err(_) => respond_error(request, 400, "invalid pixel counter value"),
            }
        }
        (_, p) if p.starts_with("/watchpoints/ppu-mode/") => {
            let mode_str = &p["/watchpoints/ppu-mode/".len()..];
            let mode = match mode_str {
                "hblank" | "0" => Some(Mode::HorizontalBlank),
                "vblank" | "1" => Some(Mode::VerticalBlank),
                "oam_scan" | "2" => Some(Mode::OamScan),
                "drawing" | "3" => Some(Mode::Drawing),
                _ => None,
            };
            match mode {
                Some(mode) => {
                    watchpoint_edit(gbdbg, request, method, WatchCondition::PpuMode(mode))
                }
                None => respond_error(
                    request,
                    400,
                    "invalid mode: use hblank/vblank/oam_scan/drawing or 0/1/2/3",
                ),
            }
        }
        _ => return Dispatch::Declined(request),
    }
    Dispatch::Handled
}

// --- MCP extension ------------------------------------------------------------

/// The Game Boy family's MCP tools: the two model-specific views worth a text
/// tool of their own. The HTTP routes remain the full model-specific surface.
#[cfg(feature = "mcp")]
pub mod mcp {
    use super::*;
    use crate::mcp::{McpExtension, Tool, ToolOutcome, text};
    use crate::session::Session;
    use serde_json::json;

    /// Cap on dots advanced by a single `gb_step_dot`.
    const MAX_DOT_COUNT: usize = 1_000_000;

    enum GbWhich {
        Dmg,
        Cgb,
    }

    fn which(session: &mut Session) -> Option<GbWhich> {
        let any = session.debugger_mut().as_any_mut();
        if any.is::<GbDebugger<Dmg>>() {
            Some(GbWhich::Dmg)
        } else if any.is::<GbDebugger<Cgb>>() {
            Some(GbWhich::Cgb)
        } else {
            None
        }
    }

    pub fn extension() -> McpExtension {
        McpExtension { tools, call }
    }

    fn tools(session: &mut Session) -> Vec<Tool> {
        if which(session).is_none() {
            return Vec::new();
        }
        vec![
            Tool {
                name: "gb_ppu_state",
                description: "Game Boy PPU state: LCDC and STAT decoded, LY/LX/LYC, scroll, \
                              window, and the three palettes."
                    .into(),
                input_schema: json!({
                    "type": "object", "properties": {}, "additionalProperties": false
                }),
            },
            Tool {
                name: "gb_step_dot",
                description: format!(
                    "Advance the console by dots (T-cycles); count default 1, max \
                     {MAX_DOT_COUNT}. Reports the pixel-pipeline state."
                ),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "count": { "type": "integer", "minimum": 1, "maximum": MAX_DOT_COUNT }
                    },
                }),
            },
        ]
    }

    fn call(session: &mut Session, name: &str, args: &Value) -> Option<ToolOutcome> {
        match name {
            "gb_ppu_state" => Some(ppu_state_tool(session)),
            "gb_step_dot" => Some(step_dot_tool(session, args)),
            _ => None,
        }
    }

    fn ppu_state_tool(session: &mut Session) -> ToolOutcome {
        let value = match which(session) {
            Some(GbWhich::Dmg) => serde_json::to_value(ppu_state(dmg(session).console())),
            Some(GbWhich::Cgb) => serde_json::to_value(ppu_state(cgb(session).console())),
            None => return Err("not a Game Boy session".into()),
        }
        .map_err(|error| error.to_string())?;
        text(serde_json::to_string_pretty(&value).unwrap())
    }

    fn step_dot_tool(session: &mut Session, args: &Value) -> ToolOutcome {
        let count = match args.get("count") {
            None | Some(Value::Null) => 1,
            Some(value) => value
                .as_u64()
                .map(|n| (n as usize).clamp(1, MAX_DOT_COUNT))
                .ok_or("count must be an integer")?,
        };
        let value = match which(session) {
            Some(GbWhich::Dmg) => {
                let gbdbg = dmg(session);
                for _ in 0..count {
                    gbdbg.step_tcycle();
                }
                pipeline_state(gbdbg.console())
            }
            Some(GbWhich::Cgb) => {
                let gbdbg = cgb(session);
                for _ in 0..count {
                    gbdbg.step_tcycle();
                }
                pipeline_state(gbdbg.console())
            }
            None => return Err("not a Game Boy session".into()),
        };
        text(serde_json::to_string_pretty(&value).unwrap())
    }
}

fn watchpoint_address<M: ConsoleUi>(
    gbdbg: &mut GbDebugger<M>,
    request: Request,
    method: &Method,
    addr_str: &str,
    build: impl Fn(u16) -> WatchCondition,
) {
    match u16::from_str_radix(addr_str, 16) {
        Ok(addr) => watchpoint_edit(gbdbg, request, method, build(addr)),
        Err(_) => respond_error(request, 400, "invalid hex address"),
    }
}

fn watchpoint_edit<M: ConsoleUi>(
    gbdbg: &mut GbDebugger<M>,
    request: Request,
    method: &Method,
    condition: WatchCondition,
) {
    match method {
        Method::Put => {
            gbdbg.add_watchpoint(condition.clone());
            respond_json(
                request,
                serde_json::json!({ "added": watchpoint_json(&condition) }),
            );
        }
        Method::Delete => {
            gbdbg.remove_watchpoint(&condition);
            respond_json(
                request,
                serde_json::json!({ "removed": watchpoint_json(&condition) }),
            );
        }
        _ => respond_error(request, 405, "method not allowed"),
    }
}

// --- inspection views (over the live console) ---------------------------------

fn cpu_state<M: Model>(gb: &Console<M>) -> CpuState {
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
        pc: cpu.ir_address,
        flags: FlagsState {
            zero: cpu.flags.contains(Flags::ZERO),
            negative: cpu.flags.contains(Flags::NEGATIVE),
            half_carry: cpu.flags.contains(Flags::HALF_CARRY),
            carry: cpu.flags.contains(Flags::CARRY),
        },
        ime: cpu.interrupts_enabled(),
        halted: cpu.halt.state != missingno_gb::cpu::HaltState::Running,
    }
}

fn disassemble<M: Model>(gb: &Console<M>, count: usize) -> Vec<InstructionEntry> {
    let pc = gb.cpu().ir_address;
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

fn ppu_state<M: Model>(gb: &Console<M>) -> PpuState {
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
                ppu::types::tiles::TileAddressMode::Block0Block1 => "8000",
                ppu::types::tiles::TileAddressMode::Block2Block1 => "8800",
            },
            bg_tile_map: control.background_tile_map().0,
            sprite_size: match control.sprite_size() {
                ppu::types::sprites::SpriteSize::Single => "8x8",
                ppu::types::sprites::SpriteSize::Double => "8x16",
            },
            sprites_enabled: control.sprites_enabled(),
            bg_and_window_enabled: control.background_and_window_enabled(),
        },
        stat: StatState {
            raw: ppu.read_register(ppu::Register::Status),
            mode: match mode {
                Mode::HorizontalBlank => "hblank",
                Mode::VerticalBlank => "vblank",
                Mode::OamScan => "oam_scan",
                Mode::Drawing => "drawing",
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

fn pipeline_state<M: Model>(gb: &Console<M>) -> Value {
    let ppu = gb.ppu();
    match ppu.pipeline_state() {
        Some(snap) => serde_json::json!({
            "pixel_counter": snap.pixel_counter,
            "rendering_active": snap.rendering_active,
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
                Some(ppu::SpriteFetchPhase::FetchingData) => Value::String("fetching_data".into()),
                None => Value::Null,
            },
            "sprite_tile_data": match snap.sprite_tile_data {
                Some((low, high)) => serde_json::json!({"low": low, "high": high}),
                None => Value::Null,
            },
            "lcd_x": snap.lcd_x,
            "fetcher_step": format!("fetch_counter={}", snap.fetch_counter),
            "window_hit": snap.window_hit,
            "pixel_gate": snap.pixel_gate,
            "fine_scroll_match": snap.fine_scroll_match,
            "fetcher_idle_stage_3": snap.fetcher_idle_stage_3,
            "fetcher_ready": snap.fetcher_ready,
            "wx_triggered": snap.wx_triggered,
            "video_clock": snap.video_clock,
            "scan_done": snap.scan_done,
            "scan_done_prev": snap.scan_done_prev,
        }),
        None => Value::Null,
    }
}

fn palette_breakdown(raw: u8) -> PaletteState {
    PaletteState {
        raw,
        colors: [raw & 3, (raw >> 2) & 3, (raw >> 4) & 3, (raw >> 6) & 3],
    }
}

fn sprites_state<M: Model>(gb: &Console<M>) -> Vec<SpriteState> {
    let ppu = gb.ppu();
    let sprite_size = ppu.control().sprite_size();
    (0..40)
        .map(|i| {
            let sprite = ppu.sprite(SpriteId(i));
            let x = sprite.position.x as i16 - 8;
            let y = sprite.position.y as i16 - 16;
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
                cgb_palette: sprite.attributes.color_palette(),
                cgb_bank: sprite.attributes.vram_bank(),
                visible: sprite.position.on_screen_x() && sprite.position.on_screen_y(sprite_size),
            }
        })
        .collect()
}

fn interrupts_state<M: Model>(gb: &Console<M>) -> InterruptsState {
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

fn vram_state<M: Model>(gb: &Console<M>, bank: u8) -> Value {
    let vram = gb.vram().bank(bank);
    let mut tile_blocks = Vec::with_capacity(3);
    for block_id in 0..3u8 {
        let block = vram.tile_block(ppu::types::tiles::TileBlockId(block_id));
        let base_addr = 0x8000u16 + block_id as u16 * 0x800;
        let mut tiles = Vec::with_capacity(128);
        for tile_idx in 0..128u8 {
            let tile = block.tile(ppu::types::tiles::TileIndex(tile_idx));
            let offset = tile_idx as usize * 16;
            let raw: Vec<u8> = block.data[offset..offset + 16].to_vec();
            let hex: Vec<String> = raw.iter().map(|b| format!("{b:02x}")).collect();
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
        let tile_map = vram.tile_map(ppu::types::tiles::TileMapId(map_id));
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

fn timers_state<M: Model>(gb: &Console<M>) -> TimersState {
    let timers = gb.timers();
    let div = timers.read_register(missingno_gb::timers::Register::Divider);
    let tima = timers.read_register(missingno_gb::timers::Register::Counter);
    let tma = timers.read_register(missingno_gb::timers::Register::Modulo);
    let tac = timers.read_register(missingno_gb::timers::Register::Control);
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

fn trace_apu<M: ConsoleUi>(gbdbg: &mut GbDebugger<M>, n: usize) -> Value {
    fn snapshot<M: Model>(console: &Console<M>, step: usize, phase: &str) -> Value {
        let cpu = console.cpu();
        let audio = console.audio();
        let ch3 = &audio.channels().ch3;
        serde_json::json!({
            "step": step,
            "phase": phase,
            "pc": cpu.ir_address,
            "a": cpu.a,
            "master_enabled": audio.enabled(),
            "ch3_2mhz": ch3.ch3_2mhz,
            "trigger_bit_latch": ch3.trigger_sync.bit_latch,
            "trigger_armed": ch3.trigger_sync.armed,
            "ch3_restart": ch3.trigger_sync.restart,
            "trigger_self_clear": ch3.trigger_sync.self_clear,
            "ch3_frst": ch3.ch3_frst,
            "ch3_fdis": ch3.ch3_fdis,
            "data_latch_sync_1": ch3.wave_data_latch.sync_1,
            "data_latch_sync_2": ch3.wave_data_latch.sync_2,
            "wave_data_latch": ch3.wave_data_latch.latched,
            "wave_data_latch_extended": ch3.wave_data_latch.extended,
            "wave_position": ch3.wave_position,
            "frequency_timer": ch3.frequency_timer,
            "period": ch3.period.0,
            "enabled": ch3.enabled.enabled,
            "dac_enabled": ch3.dac_enabled,
            "ram": ch3.ram.to_vec(),
        })
    }

    let mut rows = Vec::with_capacity(n + 1);
    rows.push(snapshot(gbdbg.console(), 0, "boundary"));
    for step in 1..=n {
        gbdbg.step_tcycle();
        rows.push(snapshot(gbdbg.console(), step, "tcycle"));
    }
    Value::Array(rows)
}

fn audio_state<M: Model>(gb: &Console<M>) -> Value {
    let audio = gb.audio();
    let channels = audio.channels();
    let ch1 = &channels.ch1;
    let ch2 = &channels.ch2;
    let ch3 = &channels.ch3;
    let ch4 = &channels.ch4;

    let enabled_json = |e: &missingno_gb::audio::channels::Enabled| {
        serde_json::json!({
            "enabled": e.enabled,
            "output_left": e.output_left,
            "output_right": e.output_right,
        })
    };

    serde_json::json!({
        "master_enabled": audio.enabled(),
        "nr50": audio.nr50(),
        "frame_sequencer_step": audio.frame_sequencer_step(),
        "prev_div_apu_bit": audio.prev_div_apu_bit(),

        "ch1": {
            "enabled": enabled_json(&ch1.enabled),
            "sweep": ch1.sweep.0,
            "waveform_and_initial_length": ch1.waveform_and_initial_length.0,
            "volume_and_envelope": ch1.volume_and_envelope.0,
            "length_enabled": ch1.length.enabled,
            "length_counter": ch1.length.counter,
            "period": ch1.period.0,
            "prescaler_counter": audio.channel_clock_counter(),
            "divider_counter": ch1.divider.counter,
            "wave_duty_position": ch1.wave_duty_position,
            "pwm_latch": ch1.pwm_latch,
            "pending_trigger_sync": ch1.pending_reload as u8,
            "divider_load_settle": ch1.divider_load_settle,
            "current_volume": ch1.envelope.volume,
            "envelope_timer": ch1.envelope.timer,
            "envelope_stopped": ch1.envelope.stopped,
            "shadow_frequency": ch1.shadow_frequency,
            "sweep_timer": ch1.sweep_timer,
            "sweep_enabled": ch1.sweep_enabled,
            "sweep_negate_used": ch1.sweep_negate_used,
        },

        "ch2": {
            "enabled": enabled_json(&ch2.enabled),
            "waveform_and_initial_length": ch2.waveform_and_initial_length.0,
            "volume_and_envelope": ch2.volume_and_envelope.0,
            "length_enabled": ch2.length.enabled,
            "length_counter": ch2.length.counter,
            "period": ch2.period.0,
            "prescaler_counter": audio.channel_clock_counter(),
            "divider_counter": ch2.divider.counter,
            "wave_duty_position": ch2.wave_duty_position,
            "pwm_latch": ch2.pwm_latch,
            "pending_trigger_sync": ch2.pending_reload as u8,
            "divider_load_settle": ch2.divider_load_settle,
            "current_volume": ch2.envelope.volume,
            "envelope_timer": ch2.envelope.timer,
            "envelope_stopped": ch2.envelope.stopped,
        },

        "ch3": {
            "enabled": enabled_json(&ch3.enabled),
            "dac_enabled": ch3.dac_enabled,
            "volume": ch3.volume.0,
            "length_enabled": ch3.length.enabled,
            "length_counter": ch3.length.counter,
            "period": ch3.period.0,
            "frequency_timer": ch3.frequency_timer,
            "wave_position": ch3.wave_position,
            "ch3_2mhz": ch3.ch3_2mhz,
            "trigger_bit_latch": ch3.trigger_sync.bit_latch,
            "trigger_armed": ch3.trigger_sync.armed,
            "ch3_restart": ch3.trigger_sync.restart,
            "trigger_self_clear": ch3.trigger_sync.self_clear,
            "ch3_frst": ch3.ch3_frst,
            "data_latch_sync_1": ch3.wave_data_latch.sync_1,
            "data_latch_sync_2": ch3.wave_data_latch.sync_2,
            "wave_data_latch": ch3.wave_data_latch.latched,
            "wave_data_latch_extended": ch3.wave_data_latch.extended,
            "ch3_fdis": ch3.ch3_fdis,
            "ram": ch3.ram.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>(),
        },

        "ch4": {
            "enabled": enabled_json(&ch4.enabled),
            "volume_and_envelope": ch4.volume_and_envelope.0,
            "length_enabled": ch4.length.enabled,
            "length_counter": ch4.length.counter,
            "frequency_and_randomness": ch4.frequency_and_randomness.0,
            "divider": ch4.divider,
            "lfsr": format!("{:04x}", ch4.lfsr),
            "current_volume": ch4.envelope.volume,
            "envelope_timer": ch4.envelope.timer,
            "envelope_stopped": ch4.envelope.stopped,
        },
    })
}

// --- watchpoint JSON (over WatchCondition) ------------------------------------

fn watchpoint_json(condition: &WatchCondition) -> Value {
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
    let json: Value = serde_json::from_str(body).map_err(|e| format!("invalid JSON: {e}"))?;
    parse_watchpoint_json(&json)
}

fn parse_watchpoint_json(json: &Value) -> Result<WatchCondition, String> {
    let typ = json["type"].as_str().ok_or("missing \"type\" field")?;
    match typ {
        "bus_read" => Ok(WatchCondition::BusRead {
            address: parse_hex_field(json, "address")?,
        }),
        "bus_write" => Ok(WatchCondition::BusWrite {
            address: parse_hex_field(json, "address")?,
        }),
        "dma_read" => Ok(WatchCondition::DmaRead {
            address: parse_hex_field(json, "address")?,
        }),
        "dma_write" => Ok(WatchCondition::DmaWrite {
            address: parse_hex_field(json, "address")?,
        }),
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

fn parse_hex_field(json: &Value, field: &str) -> Result<u16, String> {
    let s = json[field]
        .as_str()
        .ok_or(format!("missing \"{field}\" field"))?;
    u16::from_str_radix(s, 16).map_err(|_| format!("invalid hex in \"{field}\": {s}"))
}

// --- bitmap rendering ---------------------------------------------------------

fn rgba_to_rgb(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect()
}

fn write_bmp(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    let row_stride = ((width * 3 + 3) & !3) as usize;
    let pixel_data_size = row_stride * height as usize;
    let file_size = 54 + pixel_data_size;

    let mut bmp = Vec::with_capacity(file_size);

    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&(file_size as u32).to_le_bytes());
    bmp.extend_from_slice(&0u16.to_le_bytes());
    bmp.extend_from_slice(&0u16.to_le_bytes());
    bmp.extend_from_slice(&54u32.to_le_bytes());

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

    let padding = row_stride - width as usize * 3;
    for y in (0..height).rev() {
        let row_start = (y * width) as usize * 3;
        for rgb in pixels[row_start..row_start + width as usize * 3].chunks_exact(3) {
            bmp.extend_from_slice(&[rgb[2], rgb[1], rgb[0]]);
        }
        bmp.extend(std::iter::repeat_n(0u8, padding));
    }

    bmp
}

/// Renders all 384 tiles (3 blocks of 128) in a 16-wide grid.
fn tiles_bitmap<M: Model>(gb: &Console<M>, bank: u8) -> Vec<u8> {
    let vram = gb.vram().bank(bank);
    let greys: [u8; 4] = [0xFF, 0xAA, 0x55, 0x00];

    let cols = 16u32;
    let rows = 24u32;
    let w = cols * 8;
    let h = rows * 8;

    let mut pixels = vec![0u8; (w * h * 3) as usize];

    for block_id in 0..3u8 {
        let block = vram.tile_block(ppu::types::tiles::TileBlockId(block_id));
        for tile_idx in 0..128u8 {
            let tile = block.tile(ppu::types::tiles::TileIndex(tile_idx));
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

// --- HTTP responders ----------------------------------------------------------

fn respond_json(request: Request, body: impl Serialize) {
    let json = serde_json::to_string_pretty(&body).unwrap();
    let response =
        Response::from_string(json).with_header(header("Content-Type: application/json"));
    let _ = request.respond(response);
}

fn respond_error(request: Request, code: u16, message: &str) {
    let json = serde_json::to_string(&serde_json::json!({ "error": message })).unwrap();
    let response = Response::from_string(json)
        .with_status_code(StatusCode(code))
        .with_header(header("Content-Type: application/json"));
    let _ = request.respond(response);
}

fn respond_bmp(request: Request, bmp: Vec<u8>) {
    let response = Response::from_data(bmp).with_header(header("Content-Type: image/bmp"));
    let _ = request.respond(response);
}

fn header(text: &str) -> Header {
    text.parse::<Header>().expect("valid header literal")
}

// --- response structs (field order is wire-visible) ---------------------------

#[derive(Serialize)]
struct ScreenAscii {
    lines: Vec<String>,
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
struct SpriteState {
    id: u8,
    x: i16,
    y: i16,
    tile: u8,
    priority: &'static str,
    flip_x: bool,
    flip_y: bool,
    palette: &'static str,
    cgb_palette: u8,
    cgb_bank: u8,
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
