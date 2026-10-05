; Control for timer_postreload_tac: the TAC write one M-cycle later (M7), an
; ordinary write-caused wrap. TIMA reloads from TMA and reads $FF.
INCLUDE "common.inc"
SECTION "main", ROM0[$150]
main:
    di
    ld sp, $DFFF
    xor a
    ldh [$FFFF], a      ; IE off
    ldh [$FF07], a      ; TAC $00: timer disabled
    ldh [$FF06], a      ; TMA $00
    ld a, $FF
    ldh [$FF05], a      ; TIMA $FF
    ld bc, $FF06
    ld hl, $FF07
    ld de, $0500
    ldh [$FF04], a      ; M0: divider 0
    ld [hl], d          ; M2: TAC $05 (enable, tap bit 1)
    ld [bc], a          ; M4: TMA $FF
    nop
    ld [hl], e          ; M7: TAC $00, one M-cycle late (control)
    ldh a, [$FF05]      ; M9: TIMA
    finish $FF
