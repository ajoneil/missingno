#!/usr/bin/env python3
"""Headless Sim2600 signal-observation harness — the dmg-sim-observe.sh analog.

Runs an Atari 2600 ROM through the visual6502 transistor-level simulation of the
6507 + TIA and dumps a VCD waveform of named wires, one time step per half-clock,
for gate-level observation in GTKWave.

The 6532/RIOT is Sim2600's *behavioural* emulation (emuPIA), NOT a netlist — its
state is not gate-level ground truth. Observe CPU and TIA wires here; for RIOT
timing use the datasheet/schematics (see crates/missingno-vcs/AGENTS.md).

Invoked via scripts/sim2600-observe.sh, which clones + Py3-ports Sim2600 and runs
this with cwd = the Sim2600 checkout so `import params` and chips/*.pkl resolve.
"""

import argparse
import sys
import time

import params
from sim2600Console import Sim2600Console


def read_bus(sim, prefix, n):
    """Assemble an n-bit value from wires named <prefix>0..<prefix>{n-1} (lsb first)."""
    value = 0
    for i in range(n):
        if sim.isHighWN("%s%d" % (prefix, i)):
            value |= 1 << i
    return value


def build_probes(console, extra):
    """Return the ordered probe list: (display_name, width, sampler)."""
    cpu = console.sim6507
    tia = console.simTIA

    probes = [
        # 6507 — gate-level.
        ("cpu_clk0", 1, lambda: int(cpu.isHighWN("CLK0"))),
        ("cpu_rdy", 1, lambda: int(cpu.isHighWN("RDY"))),
        ("cpu_rw", 1, lambda: int(cpu.isHighWN("R/W"))),
        ("cpu_sync", 1, lambda: int(cpu.isHighWN("SYNC"))),
        ("cpu_ab", 16, cpu.getAddressBusValue),
        ("cpu_db", 8, cpu.getDataBusValue),
        ("cpu_pc", 16, lambda: read_bus(cpu, "PCL", 8) | (read_bus(cpu, "PCH", 8) << 8)),
        ("cpu_a", 8, lambda: read_bus(cpu, "A", 8)),
        ("cpu_x", 8, lambda: read_bus(cpu, "X", 8)),
        ("cpu_y", 8, lambda: read_bus(cpu, "Y", 8)),
        ("cpu_s", 8, lambda: read_bus(cpu, "S", 8)),
        ("cpu_p", 8, lambda: read_bus(cpu, "P", 8)),
        # TIA — gate-level. The beam-defining sync/blank strobes and the
        # colour/luminance output nodes.
        ("tia_vsync", 1, lambda: int(tia.isHighWN("VSYNC"))),
        ("tia_vblank", 1, lambda: int(tia.isHighWN("VBLANK"))),
        ("tia_wsync", 1, lambda: int(tia.isHighWN("WSYNC"))),
        ("tia_rsync", 1, lambda: int(tia.isHighWN("RSYNC"))),
        ("tia_clk2", 1, lambda: int(tia.isHighWN("CLK2"))),
        ("tia_ph0", 1, lambda: int(tia.isHighWN("PH0"))),
        ("tia_col", 4, lambda: read_bus(tia, "COLCNT_T", 4)),
        (
            "tia_lum",
            3,
            lambda: (
                int(tia.isHighWN("L0_lowCtrl"))
                | (int(tia.isHighWN("L1_lowCtrl")) << 1)
                | (int(tia.isHighWN("L2_lowCtrl")) << 2)
            ),
        ),
    ]

    # Arbitrary single-bit probes: --extra-wires cpu:NAME,tia:NAME — for the
    # "name the specific wire and half-clock" workflow.
    for spec in extra:
        spec = spec.strip()
        if not spec:
            continue
        chip, _, wire = spec.partition(":")
        sim = {"cpu": cpu, "tia": tia}.get(chip)
        if sim is None:
            raise SystemExit("extra wire %r must be prefixed cpu: or tia:" % spec)
        safe = "%s_%s" % (chip, wire.replace("/", "").replace(" ", ""))
        probes.append((safe, 1, (lambda s=sim, w=wire: int(s.isHighWN(w)))))

    return probes


def vcd_ids(n):
    """Generate n compact VCD identifier codes from the printable ASCII range."""
    out, i = [], 0
    while len(out) < n:
        code, x = "", i
        while True:
            out_char = chr(33 + (x % 94))
            code = out_char + code
            x = x // 94 - 1
            if x < 0:
                break
        out.append(code)
        i += 1
    return out


def emit_vcd(path, probes, samples):
    """samples: list of (halfclock, [values]) — write a VCD, changes only."""
    ids = vcd_ids(len(probes))
    with open(path, "w") as f:
        f.write("$comment Sim2600 transistor-level 6507+TIA observation $end\n")
        f.write("$comment time unit = one half-clock (no real ps model) $end\n")
        f.write("$timescale 1 ns $end\n")
        f.write("$scope module sim2600 $end\n")
        for (name, width, _), vid in zip(probes, ids):
            f.write("$var wire %d %s %s $end\n" % (width, vid, name))
        f.write("$upscope $end\n$enddefinitions $end\n")

        prev = [None] * len(probes)
        for halfclock, values in samples:
            f.write("#%d\n" % halfclock)
            for idx, ((name, width, _), vid, val) in enumerate(zip(probes, ids, values)):
                if val == prev[idx]:
                    continue
                prev[idx] = val
                if width == 1:
                    f.write("%d%s\n" % (val & 1, vid))
                else:
                    bits = format(val & ((1 << width) - 1), "b")
                    f.write("b%s %s\n" % (bits, vid))


def main():
    ap = argparse.ArgumentParser(description="Headless Sim2600 VCD observation.")
    ap.add_argument("--rom", required=True, help="path to the .a26/.bin ROM")
    ap.add_argument("--half-clocks", type=int, default=2000, help="half-clocks to run")
    ap.add_argument("--out", required=True, help="output .vcd path")
    ap.add_argument("--sample-every", type=int, default=1, help="sample every N half-clocks")
    ap.add_argument("--extra-wires", default="", help="comma list, e.g. cpu:SYNC,tia:BL_lowCtrl")
    args = ap.parse_args()

    sys.stderr.write("Loading %s into Sim2600 (transistor-level; this is slow)...\n" % args.rom)
    console = Sim2600Console(args.rom)
    # Name the 6502 register/PC bits so they can be read by wire name.
    console.sim6507.updateWireNames(params.mos6502WireInit)

    extra = args.extra_wires.split(",") if args.extra_wires else []
    probes = build_probes(console, extra)

    samples = []
    start = time.time()
    for hc in range(args.half_clocks):
        console.advanceOneHalfClock()
        if hc % args.sample_every == 0:
            samples.append((hc, [w[2]() for w in probes]))
        if hc and hc % 500 == 0:
            rate = hc / (time.time() - start)
            sys.stderr.write("  %d/%d half-clocks (%.0f/s)\n" % (hc, args.half_clocks, rate))

    emit_vcd(args.out, probes, samples)
    sys.stderr.write(
        "Wrote %s — %d samples, %d signals (%.1fs)\n"
        % (args.out, len(samples), len(probes), time.time() - start)
    )


if __name__ == "__main__":
    main()
