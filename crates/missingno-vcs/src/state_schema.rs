//! The Atari VCS hardware state schema — the authored, hardware-named
//! description of machine state the save-state bridge and trace writer key their
//! records on. This is DATA, not capture logic.
//!
//! Unlike the Game Boy PPU, the TIA never idles: the beam free-runs, so every
//! object position counter, ÷4 ring phase, serialiser latch, motion stage,
//! collision latch, and audio-tick latch is *live* at an instruction boundary.
//! There is therefore no "reconstruct from boundary defaults" shortcut — the
//! whole die state is Tier-2a boundary-complete, captured in full. The CPU sits
//! at a fetch boundary (no micro-sequencer residue) and the φ0 grid phase is a
//! function of the captured beam position, so there is no Tier-2b residue.
//!
//! Tier 1 is the programmer-observable surface: the 6507 register file, the
//! TIA write-register-equivalent bytes (colours, patterns, NUSIZ, CTRLPF flags,
//! HMxx, enables), the readable collision and RIOT registers. Tier 2a is the
//! sub-register die state that a bit-exact boundary restore also needs. The TIA
//! and CPU names are die-derived (Sim2600); the RIOT has no die oracle, so its
//! deep fields are datasheet-grounded roles, hedged as such.
//!
//! Excluded by design: the CPU micro-sequencer (Tier-2b — a save is taken only
//! at an instruction boundary, where it is `Fetch`); the frame-assembly buffers
//! and the audio resampler window (frontend Television integration, off-chip);
//! and the TIA line output buffer (overwritten each visible scanline).

use std::sync::LazyLock;

use missingno_core::state::{
    FieldDef, FieldType, FrameSpec, MemorySpan, PixelFormat, SystemStateSchema,
};

use crate::tia::VISIBLE_CLOCKS;

use FieldType::{Bool, U8, U16};

/// Tier-1 observable fields: the 6507 register file, the readable collision and
/// RIOT registers, and the TIA write-register-equivalent bytes the die stores.
pub fn observable_fields() -> Vec<FieldDef> {
    let mut fields = vec![
        // 6507 register file.
        FieldDef::observable("a", U8, "cpu").help("accumulator"),
        FieldDef::observable("x", U8, "cpu").help("X index"),
        FieldDef::observable("y", U8, "cpu").help("Y index"),
        FieldDef::observable("s", U8, "cpu").help("stack pointer (page 1 offset)"),
        FieldDef::observable("p", U8, "cpu").help("processor status flags"),
        FieldDef::observable("pc", U16, "cpu").help("program counter"),
        // TIA colour registers (COLUP0/1, COLUPF, COLUBK).
        FieldDef::observable("color_p0", U8, "tia").help("COLUP0"),
        FieldDef::observable("color_p1", U8, "tia").help("COLUP1"),
        FieldDef::observable("color_pf", U8, "tia").help("COLUPF"),
        FieldDef::observable("color_bk", U8, "tia").help("COLUBK"),
        // Vertical control latches and CTRLPF-derived playfield flags.
        FieldDef::observable("vsync", Bool, "tia").help("VSYNC latch (bit 1)"),
        FieldDef::observable("vblank", Bool, "tia").help("VBLANK latch (bit 1)"),
        FieldDef::observable("pf_priority", Bool, "tia")
            .help("CTRLPF bit 2 — playfield over players"),
        FieldDef::observable("score_mode", Bool, "tia").help("CTRLPF bit 1 — score colours"),
        // Playfield pattern registers (PF0/PF1/PF2) and the CTRLPF mirror bit.
        FieldDef::observable("pf0", U8, "tia").help("PF0 (high nibble)"),
        FieldDef::observable("pf1", U8, "tia").help("PF1"),
        FieldDef::observable("pf2", U8, "tia").help("PF2"),
        FieldDef::observable("pf_mirror", Bool, "tia").help("CTRLPF bit 0 — reflected right half"),
    ];

    // The two players' pattern/control registers.
    for (grp_new, grp_old, vdel, reflect, nusiz) in [
        (
            "p0_grp_new",
            "p0_grp_old",
            "p0_vdel",
            "p0_reflect",
            "p0_nusiz",
        ),
        (
            "p1_grp_new",
            "p1_grp_old",
            "p1_vdel",
            "p1_reflect",
            "p1_nusiz",
        ),
    ] {
        fields.push(FieldDef::observable(grp_new, U8, "tia").help("GRP live write"));
        fields.push(FieldDef::observable(grp_old, U8, "tia").help("GRP vertical-delay copy"));
        fields.push(FieldDef::observable(vdel, Bool, "tia").help("VDELP — draw the delayed copy"));
        fields.push(FieldDef::observable(reflect, Bool, "tia").help("REFP — mirror the pattern"));
        fields.push(FieldDef::observable(nusiz, U8, "tia").help("NUSIZ (player copies / size)"));
    }

    // The two missiles' enable / lock / size registers.
    for (en, lock, nusiz) in [
        ("m0_enabled", "m0_locked", "m0_nusiz"),
        ("m1_enabled", "m1_locked", "m1_nusiz"),
    ] {
        fields.push(FieldDef::observable(en, Bool, "tia").help("ENAM"));
        fields.push(
            FieldDef::observable(lock, Bool, "tia").help("RESMP — hidden, tracking its player"),
        );
        fields
            .push(FieldDef::observable(nusiz, U8, "tia").help("NUSIZ (missile width in bits 4-5)"));
    }

    // The ball's enable double-buffer, vertical delay, and CTRLPF width.
    fields.push(FieldDef::observable("bl_enabled_new", Bool, "tia").help("ENABL live"));
    fields.push(
        FieldDef::observable("bl_enabled_old", Bool, "tia").help("ENABL vertical-delay copy"),
    );
    fields.push(FieldDef::observable("bl_vdel", Bool, "tia").help("VDELBL"));
    fields
        .push(FieldDef::observable("bl_width_exp", U8, "tia").help("CTRLPF bits 4-5 — ball width"));

    // The HMxx motion registers.
    for hm in [
        "mot_hm_p0",
        "mot_hm_p1",
        "mot_hm_m0",
        "mot_hm_m1",
        "mot_hm_bl",
    ] {
        fields
            .push(FieldDef::observable(hm, U8, "tia").help("HMxx signed motion nibble (bits 4-7)"));
    }

    // The eight readable collision latches (CXxx), D7/D6 packed.
    for cx in ["cx0", "cx1", "cx2", "cx3", "cx4", "cx5", "cx6", "cx7"] {
        fields.push(FieldDef::observable(cx, U8, "tia").help("collision latch (CXxx D7/D6)"));
    }

    // The two audio channels' AUDC/AUDF/AUDV registers.
    for (c, f, v) in [
        ("ch0_control", "ch0_frequency", "ch0_volume"),
        ("ch1_control", "ch1_frequency", "ch1_volume"),
    ] {
        fields.push(FieldDef::observable(c, U8, "apu").help("AUDC waveform/tone class"));
        fields.push(FieldDef::observable(f, U8, "apu").help("AUDF frequency divisor"));
        fields.push(FieldDef::observable(v, U8, "apu").help("AUDV volume"));
    }

    // The 4 KB ROM bank paged into the window, on a banked board (else absent).
    fields.push(
        FieldDef::observable("cart_bank", U16, "cartridge")
            .help("selected 4 KB ROM bank")
            .nullable(),
    );

    // RIOT readable registers: the interval timer and both I/O ports.
    fields.push(FieldDef::observable("riot_timer", U8, "riot").help("INTIM interval-timer value"));
    fields.push(
        FieldDef::observable("riot_porta_out", U8, "riot").help("port A output register (ORA)"),
    );
    fields.push(
        FieldDef::observable("riot_porta_pins", U8, "riot").help("port A external pin levels"),
    );
    fields.push(
        FieldDef::observable("riot_porta_ddr", U8, "riot").help("port A data-direction (DDRA)"),
    );
    fields.push(
        FieldDef::observable("riot_portb_out", U8, "riot").help("port B output register (ORB)"),
    );
    fields.push(
        FieldDef::observable("riot_portb_pins", U8, "riot").help("port B external pin levels"),
    );
    fields.push(
        FieldDef::observable("riot_portb_ddr", U8, "riot").help("port B data-direction (DDRB)"),
    );

    fields
}

/// Tier-2a boundary-complete deep state: the sub-register die state a bit-exact
/// boundary restore needs — object counters and ring phases, serialiser and
/// gate latches, the HMOVE engine, per-channel audio counters, the RIOT timer
/// internals, and the console's deferred-write pipe and bus-capacitance byte.
pub fn boundary_fields() -> Vec<FieldDef> {
    let mut fields = vec![
        FieldDef::boundary("cpu_halted", Bool, "cpu").help("the 6507 is jammed (JAM opcode)"),
        // TIA line-timing spine and WSYNC/HBLANK latches.
        FieldDef::boundary("beam", U16, "tia")
            .help("HSync counter — colour clock within the line (0..228)"),
        FieldDef::boundary("hblank_release", U16, "tia")
            .help("HB-latch release the RHB decode chose (68, or 76 under HMOVE)"),
        FieldDef::boundary("cpu_ready", Bool, "tia").help("RDY — low while a WSYNC parks the CPU"),
        FieldDef::boundary("wsync_reset_hold", U8, "tia")
            .help("SHB latched-reset hold absorbing a WSYNC across the wrap"),
        // Playfield serialiser cell latch (samples PF one clock behind the write).
        FieldDef::boundary("pf_latched0", U8, "tia").help("PF0 as latched into the current cell"),
        FieldDef::boundary("pf_latched1", U8, "tia").help("PF1 as latched into the current cell"),
        FieldDef::boundary("pf_latched2", U8, "tia").help("PF2 as latched into the current cell"),
        // HMOVE motion engine.
        FieldDef::boundary("mot_arm_stage", U8, "tia")
            .help("SEC two-phase shift stage (0 idle / 1 set / 2 sampled / 3 clocked)"),
        FieldDef::boundary("mot_just_strobed", Bool, "tia")
            .help("strobe's own clock — its H@2 must not sample"),
        FieldDef::boundary("mot_ripple_active", Bool, "tia")
            .help("the 4-bit ripple counter is running"),
        FieldDef::boundary("mot_ripple", U8, "tia").help("ripple counter value (15 down to 0)"),
        FieldDef::boundary("hblank_ext_active", Bool, "tia")
            .help("HMOVE comb decode counting down"),
        FieldDef::boundary("hblank_ext_pending", U8, "tia").help("clocks until the comb arms"),
        FieldDef::boundary("hblank_ext_armed", Bool, "tia")
            .help("the RHB decode holds the blank late this line"),
        // Console deferred-write pipe and bus capacitance.
        FieldDef::boundary("last_bus_value", U8, "cpu").help("the byte the data bus still carries"),
        FieldDef::boundary("pw0_active", Bool, "tia").help("deferred TIA write slot 0 in flight"),
        FieldDef::boundary("pw0_register", U8, "tia").help("slot 0 target register"),
        FieldDef::boundary("pw0_data", U8, "tia").help("slot 0 data"),
        FieldDef::boundary("pw0_hc", U8, "tia").help("slot 0 half-clocks until it commits"),
        FieldDef::boundary("pw1_active", Bool, "tia").help("deferred TIA write slot 1 in flight"),
        FieldDef::boundary("pw1_register", U8, "tia").help("slot 1 target register"),
        FieldDef::boundary("pw1_data", U8, "tia").help("slot 1 data"),
        FieldDef::boundary("pw1_hc", U8, "tia").help("slot 1 half-clocks until it commits"),
        // Input latches and paddle charge state.
        FieldDef::boundary("trig0", Bool, "tia").help("INPT4 trigger pressed (pin low)"),
        FieldDef::boundary("trig1", Bool, "tia").help("INPT5 trigger pressed (pin low)"),
        FieldDef::boundary("trig_latch_enabled", Bool, "tia")
            .help("VBLANK bit 6 — latch the triggers"),
        FieldDef::boundary("trig_latch0", Bool, "tia").help("INPT4 latch level"),
        FieldDef::boundary("trig_latch1", Bool, "tia").help("INPT5 latch level"),
        FieldDef::boundary("pot_dumped", Bool, "tia")
            .help("VBLANK bit 7 — pot capacitors grounded"),
    ];

    // Per-object position counter, ÷4 ring phase, and START-pending latch.
    for (pos, ring, pend) in [
        ("p0_position", "p0_ring_phase", "p0_start_pending"),
        ("p1_position", "p1_ring_phase", "p1_start_pending"),
        ("m0_position", "m0_ring_phase", "m0_start_pending"),
        ("m1_position", "m1_ring_phase", "m1_start_pending"),
        ("bl_position", "bl_ring_phase", "bl_start_pending"),
    ] {
        fields.push(FieldDef::boundary(pos, U8, "tia").help("÷4 position count (0..40)"));
        fields.push(FieldDef::boundary(ring, U8, "tia").help("÷4 ring sub-phase (0..3)"));
        fields.push(FieldDef::boundary(pend, Bool, "tia").help("one-wrap START-pending latch"));
    }

    // Player serialiser scan (nullable — absent between draws).
    for (active, lead, bit, clocks, lag) in [
        (
            "p0_scan_active",
            "p0_scan_lead",
            "p0_scan_bit",
            "p0_scan_clocks",
            "p0_scan_lag",
        ),
        (
            "p1_scan_active",
            "p1_scan_lead",
            "p1_scan_bit",
            "p1_scan_clocks",
            "p1_scan_lag",
        ),
    ] {
        fields.push(FieldDef::boundary(active, Bool, "tia").help("a serialiser scan is in flight"));
        fields.push(
            FieldDef::boundary(lead, U8, "tia")
                .help("MOTCK edges until bit 0 presents")
                .nullable(),
        );
        fields.push(
            FieldDef::boundary(bit, U8, "tia")
                .help("the walked pattern bit (0..8)")
                .nullable(),
        );
        fields.push(
            FieldDef::boundary(clocks, U8, "tia")
                .help("clocks left on the current bit")
                .nullable(),
        );
        fields.push(
            FieldDef::boundary(lag, U8, "tia")
                .help("stretched serial-clock lag")
                .nullable(),
        );
    }

    // Missile width gate and reset-decode hold.
    for (lead, width, hold) in [
        ("m0_gate_lead", "m0_gate_width", "m0_reset_hold"),
        ("m1_gate_lead", "m1_gate_width", "m1_reset_hold"),
    ] {
        fields.push(FieldDef::boundary(lead, U8, "tia").help("width-gate select-network lead"));
        fields.push(FieldDef::boundary(width, U8, "tia").help("lit width remaining"));
        fields.push(
            FieldDef::boundary(hold, Bool, "tia").help("reset strobe gripping the START decode"),
        );
    }

    // Ball width gate.
    fields
        .push(FieldDef::boundary("bl_gate_lead", U8, "tia").help("width-gate select-network lead"));
    fields.push(FieldDef::boundary("bl_gate_width", U8, "tia").help("lit width remaining"));

    // HMOVE per-object more-movement latches and captured HM values.
    for m in [
        "mot_more_p0",
        "mot_more_p1",
        "mot_more_m0",
        "mot_more_m1",
        "mot_more_bl",
    ] {
        fields.push(FieldDef::boundary(m, Bool, "tia").help("more-movement latch armed"));
    }
    for c in [
        "mot_cap_p0",
        "mot_cap_p1",
        "mot_cap_m0",
        "mot_cap_m1",
        "mot_cap_bl",
    ] {
        fields.push(FieldDef::boundary(c, U8, "tia").help("HM value captured at the last H@2"));
    }
    // Per-object motion-clock seam lookahead.
    for s in ["seam_p0", "seam_p1", "seam_m0", "seam_m1", "seam_bl"] {
        fields.push(
            FieldDef::boundary(s, Bool, "tia")
                .help("merged-stuff serialiser preview for the next clock"),
        );
    }

    // Paddle knob positions (quantised) and RC-charge countdowns.
    for (p, cd) in [
        ("pot0_position", "pot0_countdown"),
        ("pot1_position", "pot1_countdown"),
        ("pot2_position", "pot2_countdown"),
        ("pot3_position", "pot3_countdown"),
    ] {
        fields.push(FieldDef::boundary(p, U16, "tia").help("knob position, 0..65535"));
        fields.push(FieldDef::boundary(cd, U16, "tia").help("RC-charge countdown in scanlines"));
    }

    // Per-channel audio counters and phase-latches (die-derived).
    for (div, pulse, noise, en, fb, tap, adv) in [
        (
            "ch0_divider",
            "ch0_pulse",
            "ch0_noise",
            "ch0_enable",
            "ch0_noise_feedback",
            "ch0_noise_tap",
            "ch0_advance",
        ),
        (
            "ch1_divider",
            "ch1_pulse",
            "ch1_noise",
            "ch1_enable",
            "ch1_noise_feedback",
            "ch1_noise_tap",
            "ch1_advance",
        ),
    ] {
        fields.push(FieldDef::boundary(div, U8, "apu").help("AUDF ÷(N+1) divider count"));
        fields.push(FieldDef::boundary(pulse, U8, "apu").help("4-bit pulse counter"));
        fields.push(FieldDef::boundary(noise, U8, "apu").help("5-bit noise LFSR"));
        fields.push(
            FieldDef::boundary(en, Bool, "apu").help("divider clock-enable latched at sample"),
        );
        fields.push(FieldDef::boundary(fb, Bool, "apu").help("noise shift-in latched at sample"));
        fields.push(FieldDef::boundary(tap, Bool, "apu").help("buffered noise tap (N2536)"));
        fields.push(
            FieldDef::boundary(adv, Bool, "apu")
                .help("pulse-hold decision latched at sample (N1530)"),
        );
    }

    // RIOT timer internals and PA7 edge detector — datasheet roles (no die oracle).
    fields.push(
        FieldDef::boundary("riot_interval", U16, "riot").help("prescaler divisor (1/8/64/1024)"),
    );
    fields.push(
        FieldDef::boundary("riot_prescaler", U16, "riot")
            .help("prescaler count toward the divisor"),
    );
    fields.push(
        FieldDef::boundary("riot_timer_phase", U8, "riot")
            .help("0 counting / 1 underflowed-this-cycle / 2 free-running"),
    );
    fields.push(FieldDef::boundary("riot_pa7_flag", Bool, "riot").help("PA7 edge-detect flag"));
    fields.push(
        FieldDef::boundary("riot_pa7_pos_edge", Bool, "riot")
            .help("PA7 detect polarity (positive edge)"),
    );

    fields
}

/// The VCS RAM regions a save state carries. ROM comes from the cartridge; cart
/// RAM is board-dependent and travels only when the board has any.
pub fn memory_spans() -> Vec<MemorySpan> {
    vec![
        MemorySpan::addressable("riot_ram", 0x0080, 0x80).help("RIOT 128-byte RAM"),
        // Cart RAM is banked/paged and its size is board-dependent, so it is an
        // off-bus linear region carried only when the board exposes one.
        MemorySpan::off_bus("cart_ram", 0)
            .optional()
            .help("linear cart RAM (board-dependent size)"),
        // A multi-slot board's bank/slot selection as an opaque per-board blob —
        // the boards a single `cart_bank` field cannot describe.
        MemorySpan::off_bus("cart_bank_state", 0)
            .optional()
            .help("board-dependent bank/slot selection"),
    ]
}

/// The VCS framebuffer: 160 visible clocks wide, emergent height (VSYNC-delimited
/// field), TIA colour indices into the region palette.
pub fn frame() -> FrameSpec {
    FrameSpec {
        width: VISIBLE_CLOCKS as u32,
        height: None,
        format: PixelFormat::Indexed8,
    }
}

static VCS_SCHEMA: LazyLock<SystemStateSchema> = LazyLock::new(|| {
    let mut fields = observable_fields();
    fields.extend(boundary_fields());
    SystemStateSchema {
        system: "vcs",
        fields,
        memory: memory_spans(),
        frame: frame(),
    }
});

/// The Atari VCS hardware state schema.
pub fn vcs_state_schema() -> &'static SystemStateSchema {
    &VCS_SCHEMA
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_is_well_formed() {
        assert_eq!(vcs_state_schema().check(), Ok(()));
    }

    #[test]
    fn every_field_has_a_tier_and_subsystem() {
        for field in &vcs_state_schema().fields {
            assert!(!field.name.is_empty());
            assert!(!field.subsystem.is_empty());
        }
    }

    #[test]
    fn carries_the_cpu_register_file_and_ram() {
        let schema = vcs_state_schema();
        for name in ["a", "x", "y", "s", "p", "pc"] {
            assert!(schema.field(name).is_some(), "missing {name}");
        }
        assert!(schema.span("riot_ram").is_some());
        assert!(schema.span("cart_ram").is_some());
    }
}
