use missingno_gb::cpu::flags::Flags;
use missingno_gb::cpu::instructions::Instruction;
use missingno_gb::debugger::Debugger;
use missingno_gb::debugger::instructions::InstructionsIterator;
use missingno_gb::interrupts;
use missingno_gb::ppu;
use missingno_gb::ppu::memory::Vram;
use missingno_gb::ppu::types::sprites::{Attributes, SpriteId};
use missingno_gb::{Console, Model};
use serde::Serialize;

pub(super) fn cpu_state<M: Model>(gb: &Console<M>) -> CpuState {
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

pub(super) fn disassemble<M: Model>(gb: &Console<M>, count: usize) -> Vec<InstructionEntry> {
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

pub(super) fn ppu_state<M: Model>(gb: &Console<M>) -> PpuState {
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
                ppu::rendering::Mode::HorizontalBlank => "hblank",
                ppu::rendering::Mode::VerticalBlank => "vblank",
                ppu::rendering::Mode::OamScan => "oam_scan",
                ppu::rendering::Mode::Drawing => "drawing",
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

pub(super) fn pipeline_state<M: Model>(gb: &Console<M>) -> serde_json::Value {
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
                Some(ppu::SpriteFetchPhase::FetchingData) => serde_json::Value::String("fetching_data".into()),
                None => serde_json::Value::Null,
            },
            "sprite_tile_data": match snap.sprite_tile_data {
                Some((low, high)) => serde_json::json!({"low": low, "high": high}),
                None => serde_json::Value::Null,
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
        None => serde_json::Value::Null,
    }
}

fn palette_breakdown(raw: u8) -> PaletteState {
    PaletteState {
        raw,
        colors: [raw & 3, (raw >> 2) & 3, (raw >> 4) & 3, (raw >> 6) & 3],
    }
}

pub(super) fn sprites_state<M: Model>(gb: &Console<M>) -> Vec<SpriteState> {
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

pub(super) fn interrupts_state<M: Model>(gb: &Console<M>) -> InterruptsState {
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

pub(super) fn vram_state<M: Model>(gb: &Console<M>, bank: u8) -> serde_json::Value {
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

pub(super) fn timers_state<M: Model>(gb: &Console<M>) -> TimersState {
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

pub(super) fn trace_apu<M: Model>(debugger: &mut Debugger<M>, n: usize) -> serde_json::Value {
    // Capture per-T-cycle CH3 state across `n` step-tcycle calls. Used
    // by /trace-apu/{n} for side-by-side comparison against the
    // dmg-sim FST. The first row records the state BEFORE any step
    // (step=0, phase="boundary"); the remaining rows record state AFTER
    // each successive T-cycle (phase="tcycle").
    fn snapshot<M: Model>(debugger: &Debugger<M>, step: usize, phase: &str) -> serde_json::Value {
        let gb = debugger.game_boy();
        let cpu = gb.cpu();
        let audio = gb.audio();
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
    rows.push(snapshot(debugger, 0, "boundary"));
    for step in 1..=n {
        debugger.step_tcycle();
        rows.push(snapshot(debugger, step, "tcycle"));
    }
    serde_json::Value::Array(rows)
}

pub(super) fn audio_state<M: Model>(gb: &Console<M>) -> serde_json::Value {
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

#[derive(Serialize)]
pub(super) struct CpuState {
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
pub(super) struct FlagsState {
    zero: bool,
    negative: bool,
    half_carry: bool,
    carry: bool,
}

#[derive(Serialize)]
pub(super) struct InstructionEntry {
    address: String,
    text: String,
}

#[derive(Serialize)]
pub(super) struct PpuState {
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
pub(super) struct LcdcState {
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
pub(super) struct StatState {
    raw: u8,
    mode: &'static str,
    mode_number: u8,
}

#[derive(Serialize)]
pub(super) struct PaletteState {
    raw: u8,
    colors: [u8; 4],
}

#[derive(Serialize)]
pub(super) struct SpriteState {
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
pub(super) struct InterruptsState {
    ie_raw: u8,
    if_raw: u8,
    vblank: InterruptLine,
    stat: InterruptLine,
    timer: InterruptLine,
    serial: InterruptLine,
    joypad: InterruptLine,
}

#[derive(Serialize)]
pub(super) struct InterruptLine {
    enabled: bool,
    requested: bool,
}

#[derive(Serialize)]
pub(super) struct TimersState {
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
