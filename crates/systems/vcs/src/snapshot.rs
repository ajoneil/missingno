//! The VCS save-state bridge: it maps the console's internal state onto the
//! hardware-named [`SystemStateSchema`] and back. Capture reads the console into
//! a [`StateRecord`] keyed by the schema's field names; restore parses a record
//! and rebuilds the console in place at an instruction boundary.
//!
//! At a VCS instruction boundary the CPU is at a fetch boundary (no
//! micro-sequencer residue) and the φ0 grid phase follows the captured beam
//! position, so there is no Tier-2b residue — the whole die state is captured
//! and the restore is bit-exact for every scanline emitted after it. The
//! frame-assembly buffers and audio resampler window are the frontend
//! Television's off-chip integration surface and are reconstructed empty; the
//! field re-locks on the next VSYNC.

use missingno_core::state::{StateRecord, StateValue};
use missingno_core::system::StateError;

use crate::console::Vcs;
use crate::riot::RiotState;
use crate::tia::TiaState;
use crate::tia::objects::ScanState;

/// Read the whole console into a schema-keyed record.
pub fn read_state(vcs: &Vcs) -> StateRecord {
    let cpu = &vcs.cpu;
    let tia = vcs.tia.capture();
    let riot = vcs.riot.capture();
    let pending = vcs.pending_tia_writes();

    let mut r = StateRecord::new();
    match vcs.cartridge().selected_bank() {
        Some(bank) => {
            r.set("cart_bank", bank as u16);
        }
        None => {
            r.set("cart_bank", StateValue::Null);
        }
    }
    // 6507 register file.
    r.set("a", cpu.a)
        .set("x", cpu.x)
        .set("y", cpu.y)
        .set("s", cpu.s)
        .set("p", cpu.p)
        .set("pc", cpu.pc)
        .set("cpu_halted", cpu.jammed())
        .set("last_bus_value", vcs.last_bus_value());

    write_tia(&mut r, &tia);
    write_pending(&mut r, &pending);
    write_riot(&mut r, &riot);
    r
}

fn write_tia(r: &mut StateRecord, t: &TiaState) {
    // TIA scalars and CTRLPF-derived flags.
    r.set("color_p0", t.color_p0)
        .set("color_p1", t.color_p1)
        .set("color_pf", t.color_pf)
        .set("color_bk", t.color_bk)
        .set("vsync", t.vsync)
        .set("vblank", t.vblank)
        .set("pf_priority", t.playfield_priority)
        .set("score_mode", t.score_mode)
        .set("pf0", t.playfield.pf0)
        .set("pf1", t.playfield.pf1)
        .set("pf2", t.playfield.pf2)
        .set("pf_mirror", t.playfield.mirrored)
        .set("pf_latched0", t.playfield.latched[0])
        .set("pf_latched1", t.playfield.latched[1])
        .set("pf_latched2", t.playfield.latched[2]);

    // Line-timing spine and WSYNC/HBLANK latches.
    r.set("beam", t.hsync_position)
        .set("hblank_release", t.hblank_release)
        .set("cpu_ready", t.cpu_ready)
        .set("wsync_reset_hold", t.wsync_reset_hold);

    // Players.
    write_player(r, "p0", &t.player0);
    write_player(r, "p1", &t.player1);
    // Missiles.
    write_missile(r, "m0", &t.missile0);
    write_missile(r, "m1", &t.missile1);
    // Ball.
    r.set("bl_enabled_new", t.ball.enabled_new)
        .set("bl_enabled_old", t.ball.enabled_old)
        .set("bl_vdel", t.ball.vertical_delay)
        .set("bl_width_exp", t.ball.width_exponent)
        .set("bl_position", t.ball.position)
        .set("bl_ring_phase", t.ball.ring_phase)
        .set("bl_start_pending", t.ball.start_pending)
        .set("bl_gate_lead", t.ball.gate_lead)
        .set("bl_gate_width", t.ball.gate_width_left)
        .set("bl_gate_unshown", t.ball.gate_start_unshown);

    // HMOVE engine.
    let objs = ["p0", "p1", "m0", "m1", "bl"];
    for (i, o) in objs.iter().enumerate() {
        r.set(hm_field(o), t.mot_hm_values[i]);
        r.set(more_field(o), t.mot_more_movement[i]);
        r.set(cap_field(o), t.mot_captured_hm[i]);
        r.set(subsume_field(o), t.subsume_next_edge[i]);
    }
    r.set("mot_arm_stage", t.mot_arm_stage)
        .set("mot_just_strobed", t.mot_just_strobed)
        .set("mot_ripple_active", t.mot_ripple_active)
        .set("mot_ripple", t.mot_ripple)
        .set("hblank_ext_active", t.hblank_ext_pending_active)
        .set("hblank_ext_pending", t.hblank_ext_pending)
        .set("hblank_ext_armed", t.hblank_ext_armed);

    // Collisions.
    for (i, cx) in ["cx0", "cx1", "cx2", "cx3", "cx4", "cx5", "cx6", "cx7"]
        .iter()
        .enumerate()
    {
        r.set(cx, t.collisions[i]);
    }

    // Audio.
    write_channel(r, "ch0", &t.audio[0]);
    write_channel(r, "ch1", &t.audio[1]);

    // Inputs and paddles.
    r.set("trig0", t.triggers[0])
        .set("trig1", t.triggers[1])
        .set("trig_latch_enabled", t.trigger_latch_enabled)
        .set("trig_latch0", t.trigger_latches[0])
        .set("trig_latch1", t.trigger_latches[1])
        .set("pot_dumped", t.pot_dumped)
        .set("pot0_position", t.pot_position[0])
        .set("pot0_countdown", t.pot_countdown[0])
        .set("pot1_position", t.pot_position[1])
        .set("pot1_countdown", t.pot_countdown[1])
        .set("pot2_position", t.pot_position[2])
        .set("pot2_countdown", t.pot_countdown[2])
        .set("pot3_position", t.pot_position[3])
        .set("pot3_countdown", t.pot_countdown[3]);
}

fn write_player(r: &mut StateRecord, o: &str, p: &crate::tia::objects::PlayerState) {
    r.set(field(o, "grp_new"), p.graphics_new)
        .set(field(o, "grp_old"), p.graphics_old)
        .set(field(o, "vdel"), p.vertical_delay)
        .set(field(o, "reflect"), p.reflect)
        .set(field(o, "nusiz"), p.nusiz)
        .set(field(o, "position"), p.position)
        .set(field(o, "ring_phase"), p.ring_phase)
        .set(field(o, "start_pending"), p.start_pending)
        .set(field(o, "reset_hold"), p.reset_decode_hold);
    match &p.scan {
        Some(s) => {
            r.set(field(o, "scan_active"), true)
                .set(field(o, "scan_lead"), s.lead)
                .set(field(o, "scan_bit"), s.bit)
                .set(field(o, "scan_clocks"), s.clocks_left)
                .set(field(o, "scan_lag"), s.serial_lag);
        }
        None => {
            r.set(field(o, "scan_active"), false)
                .set(field(o, "scan_lead"), StateValue::Null)
                .set(field(o, "scan_bit"), StateValue::Null)
                .set(field(o, "scan_clocks"), StateValue::Null)
                .set(field(o, "scan_lag"), StateValue::Null);
        }
    }
}

fn write_missile(r: &mut StateRecord, o: &str, m: &crate::tia::objects::MissileState) {
    r.set(field(o, "enabled"), m.enabled)
        .set(field(o, "locked"), m.locked_to_player)
        .set(field(o, "nusiz"), m.nusiz)
        .set(field(o, "position"), m.position)
        .set(field(o, "ring_phase"), m.ring_phase)
        .set(field(o, "start_pending"), m.start_pending)
        .set(field(o, "gate_lead"), m.gate_lead)
        .set(field(o, "gate_width"), m.gate_width_left)
        .set(field(o, "gate_unshown"), m.gate_start_unshown)
        .set(field(o, "reset_hold"), m.reset_decode_hold);
}

fn write_channel(r: &mut StateRecord, c: &str, ch: &crate::tia::audio::ChannelState) {
    r.set(field(c, "control"), ch.control)
        .set(field(c, "frequency"), ch.frequency)
        .set(field(c, "volume"), ch.volume)
        .set(field(c, "divider"), ch.divider_count)
        .set(field(c, "pulse"), ch.pulse)
        .set(field(c, "noise"), ch.noise)
        .set(field(c, "enable"), ch.enable)
        .set(field(c, "noise_feedback"), ch.noise_feedback)
        .set(field(c, "noise_tap"), ch.noise_tap)
        .set(field(c, "advance"), ch.advance);
}

fn write_pending(r: &mut StateRecord, pending: &[(u8, u8, u8)]) {
    let slot = |r: &mut StateRecord, i: usize, a, rg, d, hc| match pending.get(i) {
        Some(&(register, data, half_clocks)) => {
            r.set(a, true)
                .set(rg, register)
                .set(d, data)
                .set(hc, half_clocks);
        }
        None => {
            r.set(a, false).set(rg, 0u8).set(d, 0u8).set(hc, 0u8);
        }
    };
    slot(r, 0, "pw0_active", "pw0_register", "pw0_data", "pw0_hc");
    slot(r, 1, "pw1_active", "pw1_register", "pw1_data", "pw1_hc");
}

fn write_riot(r: &mut StateRecord, s: &RiotState) {
    r.set("riot_timer", s.timer)
        .set("riot_porta_out", s.porta_output)
        .set("riot_porta_pins", s.porta_pins)
        .set("riot_porta_ddr", s.porta_ddr)
        .set("riot_portb_out", s.portb_output)
        .set("riot_portb_pins", s.portb_pins)
        .set("riot_portb_ddr", s.portb_ddr)
        .set("riot_interval", s.interval)
        .set("riot_prescaler", s.prescaler)
        .set("riot_timer_phase", s.timer_phase)
        .set("riot_pa7_flag", s.pa7_flag)
        .set("riot_pa7_pos_edge", s.pa7_positive_edge);
}

// ── Static field-name lookup (schema names are &'static) ──────────

/// The schema names are static; this resolves one `<obj>_<field>` name from its
/// object prefix and field suffix without allocating.
fn field(obj: &str, suffix: &str) -> &'static str {
    for (o, s, name) in FIELD_NAMES {
        if *o == obj && *s == suffix {
            return name;
        }
    }
    unreachable!("no schema field {obj}_{suffix}")
}

fn hm_field(o: &str) -> &'static str {
    match o {
        "p0" => "mot_hm_p0",
        "p1" => "mot_hm_p1",
        "m0" => "mot_hm_m0",
        "m1" => "mot_hm_m1",
        _ => "mot_hm_bl",
    }
}
fn more_field(o: &str) -> &'static str {
    match o {
        "p0" => "mot_more_p0",
        "p1" => "mot_more_p1",
        "m0" => "mot_more_m0",
        "m1" => "mot_more_m1",
        _ => "mot_more_bl",
    }
}
fn cap_field(o: &str) -> &'static str {
    match o {
        "p0" => "mot_cap_p0",
        "p1" => "mot_cap_p1",
        "m0" => "mot_cap_m0",
        "m1" => "mot_cap_m1",
        _ => "mot_cap_bl",
    }
}
fn subsume_field(o: &str) -> &'static str {
    match o {
        "p0" => "subsume_p0",
        "p1" => "subsume_p1",
        "m0" => "subsume_m0",
        "m1" => "subsume_m1",
        _ => "subsume_bl",
    }
}

/// (object prefix, field suffix, schema field name) for the per-object fields.
static FIELD_NAMES: &[(&str, &str, &str)] = &[
    ("p0", "grp_new", "p0_grp_new"),
    ("p0", "grp_old", "p0_grp_old"),
    ("p0", "vdel", "p0_vdel"),
    ("p0", "reflect", "p0_reflect"),
    ("p0", "nusiz", "p0_nusiz"),
    ("p0", "position", "p0_position"),
    ("p0", "ring_phase", "p0_ring_phase"),
    ("p0", "start_pending", "p0_start_pending"),
    ("p0", "reset_hold", "p0_reset_hold"),
    ("p0", "scan_active", "p0_scan_active"),
    ("p0", "scan_lead", "p0_scan_lead"),
    ("p0", "scan_bit", "p0_scan_bit"),
    ("p0", "scan_clocks", "p0_scan_clocks"),
    ("p0", "scan_lag", "p0_scan_lag"),
    ("p1", "grp_new", "p1_grp_new"),
    ("p1", "grp_old", "p1_grp_old"),
    ("p1", "vdel", "p1_vdel"),
    ("p1", "reflect", "p1_reflect"),
    ("p1", "nusiz", "p1_nusiz"),
    ("p1", "position", "p1_position"),
    ("p1", "ring_phase", "p1_ring_phase"),
    ("p1", "start_pending", "p1_start_pending"),
    ("p1", "reset_hold", "p1_reset_hold"),
    ("p1", "scan_active", "p1_scan_active"),
    ("p1", "scan_lead", "p1_scan_lead"),
    ("p1", "scan_bit", "p1_scan_bit"),
    ("p1", "scan_clocks", "p1_scan_clocks"),
    ("p1", "scan_lag", "p1_scan_lag"),
    ("m0", "enabled", "m0_enabled"),
    ("m0", "locked", "m0_locked"),
    ("m0", "nusiz", "m0_nusiz"),
    ("m0", "position", "m0_position"),
    ("m0", "ring_phase", "m0_ring_phase"),
    ("m0", "start_pending", "m0_start_pending"),
    ("m0", "gate_lead", "m0_gate_lead"),
    ("m0", "gate_width", "m0_gate_width"),
    ("m0", "gate_unshown", "m0_gate_unshown"),
    ("m0", "reset_hold", "m0_reset_hold"),
    ("m1", "enabled", "m1_enabled"),
    ("m1", "locked", "m1_locked"),
    ("m1", "nusiz", "m1_nusiz"),
    ("m1", "position", "m1_position"),
    ("m1", "ring_phase", "m1_ring_phase"),
    ("m1", "start_pending", "m1_start_pending"),
    ("m1", "gate_lead", "m1_gate_lead"),
    ("m1", "gate_width", "m1_gate_width"),
    ("m1", "gate_unshown", "m1_gate_unshown"),
    ("m1", "reset_hold", "m1_reset_hold"),
    ("ch0", "control", "ch0_control"),
    ("ch0", "frequency", "ch0_frequency"),
    ("ch0", "volume", "ch0_volume"),
    ("ch0", "divider", "ch0_divider"),
    ("ch0", "pulse", "ch0_pulse"),
    ("ch0", "noise", "ch0_noise"),
    ("ch0", "enable", "ch0_enable"),
    ("ch0", "noise_feedback", "ch0_noise_feedback"),
    ("ch0", "noise_tap", "ch0_noise_tap"),
    ("ch0", "advance", "ch0_advance"),
    ("ch1", "control", "ch1_control"),
    ("ch1", "frequency", "ch1_frequency"),
    ("ch1", "volume", "ch1_volume"),
    ("ch1", "divider", "ch1_divider"),
    ("ch1", "pulse", "ch1_pulse"),
    ("ch1", "noise", "ch1_noise"),
    ("ch1", "enable", "ch1_enable"),
    ("ch1", "noise_feedback", "ch1_noise_feedback"),
    ("ch1", "noise_tap", "ch1_noise_tap"),
    ("ch1", "advance", "ch1_advance"),
];

/// The RAM regions a save state carries, keyed by schema span name.
pub fn capture_memory(vcs: &Vcs) -> Vec<(&'static str, Vec<u8>)> {
    let mut regions = vec![("riot_ram", vcs.riot.ram.to_vec())];
    let ram_len = vcs.cartridge().ram_len();
    if ram_len > 0 {
        let ram = (0..ram_len).map(|i| vcs.cartridge().peek_ram(i)).collect();
        regions.push(("cart_ram", ram));
    }
    // A multi-slot board's bank/slot selection, which the single `cart_bank`
    // field cannot describe. Carried only when the board holds any.
    let bank_state = vcs.cartridge().bank_state();
    if !bank_state.is_empty() {
        regions.push(("cart_bank_state", bank_state));
    }
    regions
}

// ── Record readout ────────────────────────────────────────────────

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

// ── Restore ───────────────────────────────────────────────────────

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
