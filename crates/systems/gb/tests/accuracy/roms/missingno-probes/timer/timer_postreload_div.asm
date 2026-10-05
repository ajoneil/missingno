; A DIV write wraps TIMA in the M-cycle after a reload.
; M0 DIV (divider 0), M3 TAC<-$05 (tap bit 1 already high: no edge),
; M4 natural tick wraps TIMA $FF->$00, M5 reload $FF + IF, M6 DIV write drops
; tap bit 1 (divider 6), taking TIMA $FF->$00. M9 TIMA read.
; In the DMG-CPU B netlist, MUGY holds NYDU reset while MEXU loads TIMA, and
; MEXU releases just after the boundary that closes the reload M-cycle. So
; NYDU holds 0 through M6, MERY can't detect the wrap, nothing reloads, and
; TIMA reads $00. A gate-level simulation (dmg-sim) agrees, and so does a DMG,
; running a build of this probe that shows the value read on screen.
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
    ldh [$FF04], a      ; M6: divider 0 again, tap bit 1 falls
    ldh a, [$FF05]      ; M9: TIMA
    finish $00
