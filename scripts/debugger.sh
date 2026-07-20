#!/usr/bin/env bash
# Debugger API helper library for the headless emulator.
# Source this file to get dbg_* functions:  . scripts/debugger.sh
#
# These are core-agnostic: they drive the generic missingno-debugger HTTP
# transport (any Game Boy, Atari VCS, NES or Master System ROM), reading the
# console only through the Session seam routes (/status, /sections, /registers,
# /memory, /disassembly, /step*, /waveforms, /graphics, /watches).
#
# Requires: jq, curl

DBG_URL="http://127.0.0.1:3333"
DBG_PID=""
# Back-compat for callers that referenced the old variable name.
GB_URL="$DBG_URL"

# ── Server lifecycle ──────────────────────────────────────────────

dbg_start() {
  local rom_path="${1:?usage: dbg_start <rom_path> [boot_rom_path]}"
  local boot_rom_path="${2:-}"
  if [[ ! -f "$rom_path" ]]; then
    echo "error: ROM not found: $rom_path" >&2
    return 1
  fi
  if [[ -n "$boot_rom_path" && ! -f "$boot_rom_path" ]]; then
    echo "error: boot ROM not found: $boot_rom_path" >&2
    return 1
  fi

  dbg_stop 2>/dev/null

  if [[ -n "$boot_rom_path" ]]; then
    cargo run -p missingno-debugger -- "$rom_path" --boot-rom "$boot_rom_path" &>/dev/null &
  else
    cargo run -p missingno-debugger -- "$rom_path" &>/dev/null &
  fi
  DBG_PID=$!

  # Wait for the server to answer /status (up to ~60s, covering a cold build).
  local attempts=0
  while ! curl -sf "$DBG_URL/status" >/dev/null 2>&1; do
    if ! kill -0 "$DBG_PID" 2>/dev/null; then
      echo "error: server process died during startup" >&2
      return 1
    fi
    attempts=$((attempts + 1))
    if [[ $attempts -ge 120 ]]; then
      echo "error: server did not become ready in 60s" >&2
      kill "$DBG_PID" 2>/dev/null
      return 1
    fi
    sleep 0.5
  done
  echo "ready (pid $DBG_PID)"
}

dbg_stop() {
  if [[ -n "$DBG_PID" ]] && kill -0 "$DBG_PID" 2>/dev/null; then
    kill "$DBG_PID" 2>/dev/null
    wait "$DBG_PID" 2>/dev/null
    DBG_PID=""
  fi
  # Also kill any orphaned server holding the port.
  local pid
  pid=$(lsof -ti tcp:3333 2>/dev/null) || true
  if [[ -n "$pid" ]]; then
    kill "$pid" 2>/dev/null
    sleep 0.2
  fi
}

dbg_ensure() {
  curl -sf "$DBG_URL/status" >/dev/null 2>&1
}

# ── State reading ─────────────────────────────────────────────────

# Program counter, frame, title, sub-instruction tick name, last stop reason.
dbg_status() {
  curl -s "$DBG_URL/status" | jq -r \
    '"pc=\(.pc) frame=\(.frame) title=\"\(.title)\" tick=\(.tick // "none") stop=\(.stop.reason)"'
}

# The whole machine-state sidebar as JSON (sections with typed blocks).
dbg_sections() {
  curl -s "$DBG_URL/sections" | jq '.'
}

# One named section's JSON, e.g. dbg_section CPU / dbg_section PPU.
dbg_section() {
  local name="${1:?usage: dbg_section <name>}"
  curl -s "$DBG_URL/sections" | jq --arg n "$name" '.sections[] | select(.name == $n)'
}

# Every register group, each register with its rendered value, raw value and bits.
dbg_registers() {
  curl -s "$DBG_URL/registers" | jq -r \
    '.groups[] | "\(.name): " + ([.registers[] | "\(.name)=\(.value)"] | join(" "))'
}

# dbg_memory <hex-addr> [len]  — hex dump of console memory.
dbg_memory() {
  local addr="${1:?usage: dbg_memory <hex-addr> [len]}"
  local len="${2:-1}"
  curl -s "$DBG_URL/memory/$addr/$len" | jq -r '"\(.address): \(.hex | join(" "))"'
}

# Disassembly window from the current PC (or ?at=<hex>&count=<n> passthrough).
dbg_disasm() {
  local query="${1:-}"
  curl -s "$DBG_URL/disassembly${query:+?$query}" | jq -r \
    '.lines[] | "\(.address)  \(.bytes | join(" "))  \(if .kind == "data" then "db" else .text end)"'
}

# ── Stepping ──────────────────────────────────────────────────────

# dbg_step [n]  — execute n instructions (default 1), reporting the last stop.
dbg_step() {
  local n="${1:-1}"
  local out
  for _ in $(seq 1 "$n"); do
    out=$(curl -s -X POST "$DBG_URL/step")
  done
  echo "$out" | jq -r '"pc=\(.pc) frame=\(.frame) stop=\(.stop.reason)"'
}

# dbg_step_frame [n]  — run n whole frames (default 1).
dbg_step_frame() {
  local n="${1:-1}"
  local out
  for _ in $(seq 1 "$n"); do
    out=$(curl -s -X POST "$DBG_URL/step-frame")
  done
  echo "$out" | jq -r '"pc=\(.pc) frame=\(.frame) stop=\(.stop.reason)"'
}

# dbg_step_ticks [n]  — advance n sub-instruction ticks (dots / colour clocks).
# 404s on a core whose finest step is a whole instruction.
dbg_step_ticks() {
  local n="${1:-1}"
  curl -s -X POST "$DBG_URL/step-tick?count=$n" | jq -r \
    'if .error then "error: \(.error)" else "ran=\(.ran) \(.tick) pc=\(.pc) \(.video.label): \(.video.summary)" end'
}

# ── Breakpoints & watches ─────────────────────────────────────────

# dbg_break <hex-addr>  — set a PC breakpoint.
dbg_break() {
  local addr="${1:?usage: dbg_break <hex-addr>}"
  curl -s -X PUT "$DBG_URL/breakpoints/$addr" | jq -rc '.'
}

# dbg_breaks  — list set PC breakpoints.
dbg_breaks() {
  curl -s "$DBG_URL/breakpoints" | jq -c '.breakpoints'
}

# The watch keys this core exposes, with their parameter shapes.
dbg_watchables() {
  curl -s "$DBG_URL/watchables" | jq -r \
    '.watchables[] | "\(.key) [\(.param.kind)]  \(.label)"'
}

# dbg_watch <terms-json>  — add a watch. The JSON is a single term
#   {"key":"...","address":"ff40","value":3}
# or a conjunction {"terms":[ ... ]}.
dbg_watch() {
  local body="${1:?usage: dbg_watch <terms-json>}"
  curl -s -X PUT "$DBG_URL/watches" -d "$body" | jq -rc '.'
}

# dbg_watches  — list active watches.
dbg_watches() {
  curl -s "$DBG_URL/watches" | jq -c '.watches'
}

# dbg_run_until_watch [max-frames]  — step whole frames until a watch or
# breakpoint fires, or the frame budget (default 600) is exhausted.
dbg_run_until_watch() {
  local budget="${1:-600}"
  local out reason
  for _ in $(seq 1 "$budget"); do
    out=$(curl -s -X POST "$DBG_URL/step-frame")
    reason=$(echo "$out" | jq -r '.stop.reason')
    if [[ "$reason" == "watch" || "$reason" == "breakpoint" ]]; then
      echo "$out" | jq -rc '{pc, frame, stop}'
      return 0
    fi
  done
  echo "no watch/breakpoint within $budget frames" >&2
  return 1
}

# ── Media surfaces ────────────────────────────────────────────────

# Per-channel captured DAC waveforms (label, rate, depth, active, sample count).
# The first read auto-enables capture, so call again after stepping a frame.
dbg_waveforms() {
  curl -s "$DBG_URL/waveforms" | jq -r \
    'if .waveforms == null then "(this core captures no waveforms)"
     else .waveforms[] | "\(.label) @ \(.rate)Hz \(if .active then "driving" else "idle" end) — \(.levels | length) samples" end'
}

# Decoded graphics surfaces: atlas/map/object counts. The first read auto-enables
# capture; call again after stepping a frame to see it fill.
dbg_graphics() {
  curl -s "$DBG_URL/graphics" | jq -r \
    'if .graphics == null then "(this core exposes no graphics surfaces)"
     else "atlases: \([.graphics.atlases[] | "\(.label)(\(.tiles | length))"] | join(", "))\n"
        + "maps: \(.graphics.maps | length)  objects: \(if .graphics.objects then .graphics.objects.objects | length else 0 end)" end'
}

# dbg_screen <outfile.rgba>  — save the current resolved frame as raw RGBA and
# report its dimensions. Image encoding is a transport concern (the MCP
# get_frame tool hands back a PNG); the HTTP transport exposes raw pixels.
dbg_screen() {
  local out="${1:?usage: dbg_screen <outfile.rgba>}"
  local headers
  headers=$(curl -s -D - "$DBG_URL/frame/bitmap" -o "$out")
  local w h
  w=$(echo "$headers" | awk 'tolower($1) == "x-frame-width:" { print $2 }' | tr -d '\r')
  h=$(echo "$headers" | awk 'tolower($1) == "x-frame-height:" { print $2 }' | tr -d '\r')
  echo "saved $out (${w}x${h} RGBA)"
}

# The CPU-visible memory map, named by role — which spans exist, before reading
# any of them. Start addresses are hex; an off-bus region has no bus address.
dbg_regions() {
  curl -s "$DBG_URL/regions" | jq -r '.regions[] | "\(.name)\t\(.start)\t\(.len)"'
}

# dbg_save_state <path> / dbg_load_state <path> — capture the whole machine and
# restore it. Navigating to an interesting state is often the expensive part of
# an investigation: save once there, then reload instead of re-navigating.
dbg_save_state() {
  local path="${1:?usage: dbg_save_state <path>}"
  curl -s -X POST "$DBG_URL/state/save" -H 'Content-Type: application/json' \
    -d "$(jq -nc --arg p "$path" '{path: $p}')" | jq -r '.saved // .error'
}

dbg_load_state() {
  local path="${1:?usage: dbg_load_state <path>}"
  # The reported frame counts frames this session has stepped; it is session
  # bookkeeping and does not rewind with the machine.
  curl -s -X POST "$DBG_URL/state/load" -H 'Content-Type: application/json' \
    -d "$(jq -nc --arg p "$path" '{path: $p}')" |
    jq -r 'if .error then .error else "loaded (pc \(.pc // "?"))" end'
}

# dbg_control <id> <down|up>  — hold or release a console control, for reaching
# a state the ROM only enters on input. Ids follow the core's control order.
dbg_control() {
  local id="${1:?usage: dbg_control <control-id> <down|up>}"
  local action="${2:?usage: dbg_control <control-id> <down|up>}"
  local pressed=true
  [ "$action" = "up" ] && pressed=false
  curl -s -X POST "$DBG_URL/control" -H 'Content-Type: application/json' \
    -d "$(jq -nc --argjson c "$id" --argjson p "$pressed" '{control: $c, pressed: $p}')" |
    jq -r 'if .error then .error else "control \(.control) \($ARGS.named.a)" end' --arg a "$action"
}

# ── Deprecated gb_* aliases (semantics that map 1:1 to a dbg_* helper) ─────────

gb_start() {
  echo "gb_start is deprecated; use dbg_start" >&2
  dbg_start "$@"
}

gb_stop() {
  echo "gb_stop is deprecated; use dbg_stop" >&2
  dbg_stop "$@"
}

gb_run_frames() {
  echo "gb_run_frames is deprecated; use dbg_step_frame" >&2
  dbg_step_frame "$@"
}
