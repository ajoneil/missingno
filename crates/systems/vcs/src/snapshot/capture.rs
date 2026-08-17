//! Reading the console into a schema-keyed record.

use missingno_core::state::{StateRecord, StateValue};

use super::fields::{field, object_field};
use crate::console::Vcs;
use crate::riot::RiotState;
use crate::tia::TiaState;

/// Read the whole console into a schema-keyed record.
pub fn read_state(vcs: &Vcs) -> StateRecord {
    let cpu = &vcs.cpu;
    let tia = vcs.tia.capture();
    let riot = vcs.riot.capture();
    let pending = vcs.pending_tia_writes();

    let mut r = StateRecord::new();
    r.set(
        "cart_bank",
        vcs.cartridge()
            .selected_bank()
            .map_or(StateValue::Null, |bank| StateValue::from(bank as u16)),
    );
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
        r.set(object_field("mot_hm", o), t.mot_hm_values[i]);
        r.set(object_field("mot_more", o), t.mot_more_movement[i]);
        r.set(object_field("mot_cap", o), t.mot_captured_hm[i]);
        r.set(object_field("subsume", o), t.subsume_next_edge[i]);
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
