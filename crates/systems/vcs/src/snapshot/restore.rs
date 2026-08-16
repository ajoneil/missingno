//! Parsing a record back into the console.

use missingno_core::state::{StateRecord, StateValue};
use missingno_core::system::StateError;

use super::fields::{cap_field, field, hm_field, more_field, subsume_field};
use crate::console::Vcs;
use crate::riot::RiotState;
use crate::tia::TiaState;
use crate::tia::objects::ScanState;

fn int(r: &StateRecord, name: &str) -> Result<u32, StateError> {
    match r.get(name) {
        Some(StateValue::Int(v)) => Ok(*v),
        _ => Err(StateError::Corrupt),
    }
}
fn u8_of(r: &StateRecord, name: &str) -> Result<u8, StateError> {
    Ok(int(r, name)? as u8)
}
fn u16_of(r: &StateRecord, name: &str) -> Result<u16, StateError> {
    Ok(int(r, name)? as u16)
}
fn bool_of(r: &StateRecord, name: &str) -> Result<bool, StateError> {
    match r.get(name) {
        Some(StateValue::Bool(b)) => Ok(*b),
        _ => Err(StateError::Corrupt),
    }
}
/// A nullable u8 field: `None` when the record carries it as null.
fn opt_u8(r: &StateRecord, name: &str) -> Result<Option<u8>, StateError> {
    match r.get(name) {
        Some(StateValue::Int(v)) => Ok(Some(*v as u8)),
        Some(StateValue::Null) => Ok(None),
        _ => Err(StateError::Corrupt),
    }
}

/// Rebuild the console in place from a validated record and its RAM spans, at an
/// instruction boundary. Errors (never panics) on a malformed record.
pub fn restore(
    vcs: &mut Vcs,
    record: &StateRecord,
    memory: &[(String, Vec<u8>)],
) -> Result<(), StateError> {
    let region = |name: &str| {
        memory
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, d)| d.as_slice())
    };

    // Parse the whole record BEFORE mutating any subsystem — a bad or missing
    // field then leaves the console untouched rather than half-restored with the
    // CPU already reseated.
    let (a, x, y, s, p, pc, cpu_halted) = (
        u8_of(record, "a")?,
        u8_of(record, "x")?,
        u8_of(record, "y")?,
        u8_of(record, "s")?,
        u8_of(record, "p")?,
        u16_of(record, "pc")?,
        bool_of(record, "cpu_halted")?,
    );
    let tia = parse_tia(record)?;
    let riot = parse_riot(record)?;
    let pending = parse_pending(record)?;
    let last_bus_value = u8_of(record, "last_bus_value")?;
    let bank = match record.get("cart_bank") {
        Some(StateValue::Int(v)) => Some(*v as usize),
        Some(StateValue::Null) | None => None,
        _ => return Err(StateError::Corrupt),
    };

    // Everything parsed: now mutate.
    vcs.cpu.restore_boundary(a, x, y, s, p, pc, cpu_halted);
    vcs.tia.restore(&tia);
    vcs.riot.restore(&riot);
    if let Some(ram) = region("riot_ram") {
        let len = ram.len().min(vcs.riot.ram.len());
        vcs.riot.ram[..len].copy_from_slice(&ram[..len]);
    }

    // Cartridge bank(s) + cart RAM. `restore_bank` reseats the single-bank
    // boards; the board-state span carries the multi-slot boards.
    vcs.cartridge_mut().restore_bank(bank);
    if let Some(state) = region("cart_bank_state") {
        vcs.cartridge_mut().restore_bank_state(state);
    }
    if let Some(ram) = region("cart_ram") {
        vcs.cartridge_mut().restore_ram(ram);
    }

    vcs.restore_console(&pending, last_bus_value);
    Ok(())
}

fn parse_tia(r: &StateRecord) -> Result<TiaState, StateError> {
    use crate::tia::objects::{BallState, PlayfieldState};

    let objs = ["p0", "p1", "m0", "m1", "bl"];
    let mut mot_hm_values = [0u8; 5];
    let mut mot_more_movement = [false; 5];
    let mut mot_captured_hm = [0u8; 5];
    let mut subsume_next_edge = [false; 5];
    for (i, o) in objs.iter().enumerate() {
        mot_hm_values[i] = u8_of(r, hm_field(o))?;
        mot_more_movement[i] = bool_of(r, more_field(o))?;
        mot_captured_hm[i] = u8_of(r, cap_field(o))?;
        subsume_next_edge[i] = bool_of(r, subsume_field(o))?;
    }
    let mut collisions = [0u8; 8];
    for (i, cx) in ["cx0", "cx1", "cx2", "cx3", "cx4", "cx5", "cx6", "cx7"]
        .iter()
        .enumerate()
    {
        collisions[i] = u8_of(r, cx)?;
    }

    Ok(TiaState {
        vsync: bool_of(r, "vsync")?,
        vblank: bool_of(r, "vblank")?,
        cpu_ready: bool_of(r, "cpu_ready")?,
        wsync_reset_hold: u8_of(r, "wsync_reset_hold")?,
        hsync_position: u16_of(r, "beam")?,
        hblank_release: u16_of(r, "hblank_release")?,
        color_p0: u8_of(r, "color_p0")?,
        color_p1: u8_of(r, "color_p1")?,
        color_pf: u8_of(r, "color_pf")?,
        color_bk: u8_of(r, "color_bk")?,
        playfield_priority: bool_of(r, "pf_priority")?,
        score_mode: bool_of(r, "score_mode")?,
        mot_arm_stage: u8_of(r, "mot_arm_stage")?,
        mot_just_strobed: bool_of(r, "mot_just_strobed")?,
        mot_ripple_active: bool_of(r, "mot_ripple_active")?,
        mot_ripple: u8_of(r, "mot_ripple")?,
        mot_more_movement,
        mot_hm_values,
        mot_captured_hm,
        hblank_ext_pending_active: bool_of(r, "hblank_ext_active")?,
        hblank_ext_pending: u8_of(r, "hblank_ext_pending")?,
        hblank_ext_armed: bool_of(r, "hblank_ext_armed")?,
        subsume_next_edge,
        collisions,
        player0: parse_player(r, "p0")?,
        player1: parse_player(r, "p1")?,
        missile0: parse_missile(r, "m0")?,
        missile1: parse_missile(r, "m1")?,
        ball: BallState {
            enabled_new: bool_of(r, "bl_enabled_new")?,
            enabled_old: bool_of(r, "bl_enabled_old")?,
            vertical_delay: bool_of(r, "bl_vdel")?,
            width_exponent: u8_of(r, "bl_width_exp")?,
            position: u8_of(r, "bl_position")?,
            ring_phase: u8_of(r, "bl_ring_phase")?,
            start_pending: bool_of(r, "bl_start_pending")?,
            gate_lead: u8_of(r, "bl_gate_lead")?,
            gate_width_left: u8_of(r, "bl_gate_width")?,
            gate_start_unshown: bool_of(r, "bl_gate_unshown")?,
        },
        playfield: PlayfieldState {
            pf0: u8_of(r, "pf0")?,
            pf1: u8_of(r, "pf1")?,
            pf2: u8_of(r, "pf2")?,
            mirrored: bool_of(r, "pf_mirror")?,
            latched: [
                u8_of(r, "pf_latched0")?,
                u8_of(r, "pf_latched1")?,
                u8_of(r, "pf_latched2")?,
            ],
        },
        audio: [parse_channel(r, "ch0")?, parse_channel(r, "ch1")?],
        triggers: [bool_of(r, "trig0")?, bool_of(r, "trig1")?],
        trigger_latch_enabled: bool_of(r, "trig_latch_enabled")?,
        trigger_latches: [bool_of(r, "trig_latch0")?, bool_of(r, "trig_latch1")?],
        pot_position: [
            u16_of(r, "pot0_position")?,
            u16_of(r, "pot1_position")?,
            u16_of(r, "pot2_position")?,
            u16_of(r, "pot3_position")?,
        ],
        pot_countdown: [
            u16_of(r, "pot0_countdown")?,
            u16_of(r, "pot1_countdown")?,
            u16_of(r, "pot2_countdown")?,
            u16_of(r, "pot3_countdown")?,
        ],
        pot_dumped: bool_of(r, "pot_dumped")?,
    })
}

fn parse_player(r: &StateRecord, o: &str) -> Result<crate::tia::objects::PlayerState, StateError> {
    use crate::tia::objects::PlayerState;
    let scan = if bool_of(r, field(o, "scan_active"))? {
        Some(ScanState {
            lead: opt_u8(r, field(o, "scan_lead"))?.unwrap_or(0),
            bit: opt_u8(r, field(o, "scan_bit"))?.unwrap_or(0),
            clocks_left: opt_u8(r, field(o, "scan_clocks"))?.unwrap_or(0),
            serial_lag: opt_u8(r, field(o, "scan_lag"))?.unwrap_or(0),
        })
    } else {
        None
    };
    Ok(PlayerState {
        graphics_new: u8_of(r, field(o, "grp_new"))?,
        graphics_old: u8_of(r, field(o, "grp_old"))?,
        vertical_delay: bool_of(r, field(o, "vdel"))?,
        reflect: bool_of(r, field(o, "reflect"))?,
        nusiz: u8_of(r, field(o, "nusiz"))?,
        position: u8_of(r, field(o, "position"))?,
        ring_phase: u8_of(r, field(o, "ring_phase"))?,
        start_pending: bool_of(r, field(o, "start_pending"))?,
        reset_decode_hold: bool_of(r, field(o, "reset_hold"))?,
        scan,
    })
}

fn parse_missile(
    r: &StateRecord,
    o: &str,
) -> Result<crate::tia::objects::MissileState, StateError> {
    use crate::tia::objects::MissileState;
    Ok(MissileState {
        enabled: bool_of(r, field(o, "enabled"))?,
        locked_to_player: bool_of(r, field(o, "locked"))?,
        nusiz: u8_of(r, field(o, "nusiz"))?,
        position: u8_of(r, field(o, "position"))?,
        ring_phase: u8_of(r, field(o, "ring_phase"))?,
        start_pending: bool_of(r, field(o, "start_pending"))?,
        gate_lead: u8_of(r, field(o, "gate_lead"))?,
        gate_width_left: u8_of(r, field(o, "gate_width"))?,
        gate_start_unshown: bool_of(r, field(o, "gate_unshown"))?,
        reset_decode_hold: bool_of(r, field(o, "reset_hold"))?,
    })
}

fn parse_channel(r: &StateRecord, c: &str) -> Result<crate::tia::audio::ChannelState, StateError> {
    use crate::tia::audio::ChannelState;
    Ok(ChannelState {
        control: u8_of(r, field(c, "control"))?,
        frequency: u8_of(r, field(c, "frequency"))?,
        volume: u8_of(r, field(c, "volume"))?,
        divider_count: u8_of(r, field(c, "divider"))?,
        pulse: u8_of(r, field(c, "pulse"))?,
        noise: u8_of(r, field(c, "noise"))?,
        enable: bool_of(r, field(c, "enable"))?,
        noise_feedback: bool_of(r, field(c, "noise_feedback"))?,
        noise_tap: bool_of(r, field(c, "noise_tap"))?,
        advance: bool_of(r, field(c, "advance"))?,
    })
}

fn parse_riot(r: &StateRecord) -> Result<RiotState, StateError> {
    Ok(RiotState {
        timer: u8_of(r, "riot_timer")?,
        interval: u16_of(r, "riot_interval")?,
        prescaler: u16_of(r, "riot_prescaler")?,
        timer_phase: u8_of(r, "riot_timer_phase")?,
        porta_output: u8_of(r, "riot_porta_out")?,
        porta_pins: u8_of(r, "riot_porta_pins")?,
        porta_ddr: u8_of(r, "riot_porta_ddr")?,
        portb_output: u8_of(r, "riot_portb_out")?,
        portb_pins: u8_of(r, "riot_portb_pins")?,
        portb_ddr: u8_of(r, "riot_portb_ddr")?,
        pa7_flag: bool_of(r, "riot_pa7_flag")?,
        pa7_positive_edge: bool_of(r, "riot_pa7_pos_edge")?,
    })
}

fn parse_pending(r: &StateRecord) -> Result<Vec<(u8, u8, u8)>, StateError> {
    let mut pending = Vec::new();
    for (a, rg, d, hc) in [
        ("pw0_active", "pw0_register", "pw0_data", "pw0_hc"),
        ("pw1_active", "pw1_register", "pw1_data", "pw1_hc"),
    ] {
        if bool_of(r, a)? {
            pending.push((u8_of(r, rg)?, u8_of(r, d)?, u8_of(r, hc)?));
        }
    }
    Ok(pending)
}
