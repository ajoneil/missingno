; Control for timer_postreload_div: the DIV write one M-cycle later (M7), an
; ordinary write-caused wrap. TIMA reloads from TMA and reads $FF.
INCLUDE "common.inc"
SECTION "main", ROM0[$150]
main:
    di
    ld sp, $DFFF
    xor a
    ldh [$FFFF], a      ; IE off
    ldh [$FF07], a      ; TAC $00: timer disabled
    ld a, $FF
    ldh [$FF06], a      ; TMA $FF
    ldh [$FF05], a      ; TIMA $FF
    ld a, $05
    ldh [$FF04], a      ; M0: divider 0
    ldh [$FF07], a      ; M3: TAC $05
    nop
    ldh [$FF04], a      ; M7: one M-cycle late (control): an ordinary write-caused wrap
    ldh a, [$FF05]      ; M9: TIMA
    finish $FF
