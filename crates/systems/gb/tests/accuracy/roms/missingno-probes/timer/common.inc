; Shared frame for the timer probes. Result protocol as gbmicrotest:
; $FF80 = value read, $FF81 = value expected, $FF82 = $01 pass / $FF fail.
SECTION "entry", ROM0[$100]
    nop
    jp main
    ds $150 - @, 0

MACRO finish ; a = value read, \1 = expected
    ldh [$FF80], a
    ld b, \1
    ld c, a
    ld a, b
    ldh [$FF81], a
    ld a, c
    cp b
    ld a, $01
    jr z, .done\@
    ld a, $FF
.done\@:
    ldh [$FF82], a
.halt\@:
    jr .halt\@
ENDM
