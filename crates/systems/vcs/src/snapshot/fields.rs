//! Static field-name lookup (schema names are &'static).

/// The schema names are static; this resolves one `<obj>_<field>` name from its
/// object prefix and field suffix without allocating.
pub(super) fn field(obj: &str, suffix: &str) -> &'static str {
    for (o, s, name) in FIELD_NAMES {
        if *o == obj && *s == suffix {
            return name;
        }
    }
    unreachable!("no schema field {obj}_{suffix}")
}

pub(super) fn hm_field(o: &str) -> &'static str {
    match o {
        "p0" => "mot_hm_p0",
        "p1" => "mot_hm_p1",
        "m0" => "mot_hm_m0",
        "m1" => "mot_hm_m1",
        _ => "mot_hm_bl",
    }
}
pub(super) fn more_field(o: &str) -> &'static str {
    match o {
        "p0" => "mot_more_p0",
        "p1" => "mot_more_p1",
        "m0" => "mot_more_m0",
        "m1" => "mot_more_m1",
        _ => "mot_more_bl",
    }
}
pub(super) fn cap_field(o: &str) -> &'static str {
    match o {
        "p0" => "mot_cap_p0",
        "p1" => "mot_cap_p1",
        "m0" => "mot_cap_m0",
        "m1" => "mot_cap_m1",
        _ => "mot_cap_bl",
    }
}
pub(super) fn subsume_field(o: &str) -> &'static str {
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
