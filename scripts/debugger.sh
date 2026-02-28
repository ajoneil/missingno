#!/usr/bin/env bash
# Debugger API helper library for the headless emulator.
# Source this file to get gb_* functions:  . scripts/debugger.sh
#
# Requires: jq, curl

GB_URL="http://127.0.0.1:3333"
GB_PID=""

# ── Server lifecycle ──────────────────────────────────────────────

gb_start() {
  local rom_path="${1:?usage: gb_start <rom_path>}"
  if [[ ! -f "$rom_path" ]]; then
    echo "error: ROM not found: $rom_path" >&2
    return 1
  fi

  gb_stop 2>/dev/null

  cargo run -- "$rom_path" --headless &>/dev/null &
  GB_PID=$!

  # Wait for server to be ready (up to 30 seconds)
  local attempts=0
  while ! curl -sf "$GB_URL/cpu" >/dev/null 2>&1; do
    if ! kill -0 "$GB_PID" 2>/dev/null; then
      echo "error: server process died during startup" >&2
      return 1
    fi
    attempts=$((attempts + 1))
    if [[ $attempts -ge 60 ]]; then
      echo "error: server did not become ready in 30s" >&2
      kill "$GB_PID" 2>/dev/null
      return 1
    fi
    sleep 0.5
  done
  echo "ready (pid $GB_PID)"
}

gb_stop() {
  if [[ -n "$GB_PID" ]] && kill -0 "$GB_PID" 2>/dev/null; then
    kill "$GB_PID" 2>/dev/null
    wait "$GB_PID" 2>/dev/null
    GB_PID=""
  fi
  # Also kill any orphaned headless servers on the port
  local pid
  pid=$(lsof -ti tcp:3333 2>/dev/null) || true
  if [[ -n "$pid" ]]; then
    kill "$pid" 2>/dev/null
    sleep 0.2
  fi
}

gb_ensure() {
  curl -sf "$GB_URL/cpu" >/dev/null 2>&1
}

# ── Navigation ────────────────────────────────────────────────────

gb_reset() {
  curl -s -X POST "$GB_URL/reset" >/dev/null
  curl -s -X DELETE "$GB_URL/watchpoints" >/dev/null
  echo "reset"
}

gb_run_frames() {
  local n="${1:?usage: gb_run_frames <count>}"
  for _ in $(seq 1 "$n"); do
    curl -s -X POST "$GB_URL/step-frame" >/dev/null
  done
}

gb_goto() {
  local scanline="${1:?usage: gb_goto <scanline> <mode>}"
  local mode="${2:?usage: gb_goto <scanline> <mode>}"
  curl -s -X DELETE "$GB_URL/watchpoints" >/dev/null
  curl -s -X POST "$GB_URL/watchpoints" \
    -d "{\"type\":\"all\",\"conditions\":[{\"type\":\"scanline\",\"value\":$scanline},{\"type\":\"ppu_mode\",\"mode\":\"$mode\"}]}" >/dev/null
  curl -s -X POST "$GB_URL/step-frame" >/dev/null
  curl -s -X DELETE "$GB_URL/watchpoints" >/dev/null
  gb_ppu
}

gb_step_dots() {
  local n="${1:?usage: gb_step_dots <count>}"
  printf "%-4s  %-4s  %-3s  %-6s  %-5s  %-5s  %s\n" \
    "step" "dot" "pc" "loaded" "lo" "hi" "sprite"
  for i in $(seq 1 "$n"); do
    local pipe dot
    pipe=$(curl -s -X POST "$GB_URL/step-dot")
    dot=$(curl -s "$GB_URL/ppu" | jq -r '.dot')
    echo "$pipe" | jq -r --arg i "$i" --arg dot "$dot" \
      '"\($i | " " * (4 - ($i|length))) \($i)  \($dot | " " * (4 - ($dot|length)))\($dot)  \(.pixel_counter | tostring | " " * (3 - length))\(.pixel_counter)  \(if .bg_shifter.loaded then "true  " else "false " end)  \(.bg_shifter.low | tostring | " " * (5 - length))\(.bg_shifter.low)  \(.bg_shifter.high | tostring | " " * (5 - length))\(.bg_shifter.high)  \(.sprite_fetch // "none")"'
  done
}

# ── State reading ─────────────────────────────────────────────────

gb_ppu() {
  curl -s "$GB_URL/ppu" | jq -r \
    '"LY=\(.ly) dot=\(.dot) mode=\(.stat.mode_number) SCX=\(.scx) SCY=\(.scy) WX=\(.wx) WY=\(.wy) BGP=\(.bgp.colors)"'
}

gb_pipeline() {
  curl -s "$GB_URL/ppu/pipeline" | jq -r \
    '"pc=\(.pixel_counter) loaded=\(.bg_shifter.loaded) lo=\(.bg_shifter.low) hi=\(.bg_shifter.high) phase=\(.render_phase) sprite=\(.sprite_fetch // "none")"'
}

gb_cpu() {
  curl -s "$GB_URL/cpu" | jq -r \
    '"A=\(.a) B=\(.b) C=\(.c) D=\(.d) E=\(.e) H=\(.h) L=\(.l) PC=\(.pc) SP=\(.sp) IME=\(.ime) halted=\(.halted)"'
}

gb_screen_row() {
  local row="${1:?usage: gb_screen_row <row>}"
  curl -s "$GB_URL/screen" | jq -r ".pixels[$row] | map(tostring) | join(\" \")"
}

gb_sprites_on() {
  local scanline="${1:?usage: gb_sprites_on <scanline>}"
  curl -s "$GB_URL/sprites" | jq -r \
    --argjson ly "$scanline" \
    '[.[] | select(.visible and .y <= $ly and $ly < .y + 8)] |
     if length == 0 then "no sprites on scanline \($ly)"
     else .[] | "id=\(.id) x=\(.x) y=\(.y) tile=\(.tile) prio=\(.priority)"
     end'
}

gb_tile_data() {
  local tile_id="${1:?usage: gb_tile_data <tile_id> [row]}"
  local row="${2:-}"
  local addr
  addr=$(printf "%X" $((0x8000 + tile_id * 16)))
  if [[ -n "$row" ]]; then
    # Single row: 2 bytes at offset row*2
    local row_addr
    row_addr=$(printf "%X" $((0x8000 + tile_id * 16 + row * 2)))
    curl -s "$GB_URL/memory/$row_addr/2" | jq -r '
      .bytes as $b | ($b[0]) as $lo | ($b[1]) as $hi |
      [range(7;-1;-1)] | map(
        . as $bit |
        ((($hi / pow(2;.)) | floor) % 2 * 2) + ((($lo / pow(2;.)) | floor) % 2)
      ) | map(tostring) | join(" ")'
  else
    # All 8 rows
    curl -s "$GB_URL/memory/$addr/16" | jq -r '
      .bytes as $b |
      [range(8)] | map(
        . as $r | ($b[$r*2]) as $lo | ($b[$r*2+1]) as $hi |
        "row \($r): " + ([range(7;-1;-1)] | map(
          . as $bit |
          ((($hi / pow(2;.)) | floor) % 2 * 2) + ((($lo / pow(2;.)) | floor) % 2)
        ) | map(tostring) | join(" "))
      )[]'
  fi
}

gb_tile_map_row() {
  local row="${1:?usage: gb_tile_map_row <row>}"
  local addr
  addr=$(printf "%X" $((0x9800 + row * 32)))
  curl -s "$GB_URL/memory/$addr/32" | jq -r \
    '.bytes | to_entries | map("\(.key):\(.value)") | join("  ")'
}
