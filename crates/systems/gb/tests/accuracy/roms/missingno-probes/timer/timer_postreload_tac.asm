; A TAC write wraps TIMA in the M-cycle after a reload.
; TIMA wraps naturally and reloads $FF from TMA; in the very next M-cycle a
; TAC write drops the timer input (tap bit 1 high), taking TIMA $FF->$00.
; M-cycles counted from the DIV write (M0): M2 TAC<-$05, M4 TMA<-$FF (in the
; wrap M-cycle), M5 reload + IF, M6 TAC<-$00, M9 TIMA read.
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
    ldh [$FF06], a      ; TMA $00
    ld a, $FF
    ldh [$FF05], a      ; TIMA $FF
    ld bc, $FF06
    ld hl, $FF07
    ld de, $0500
    ldh [$FF04], a      ; M0: divider 0
    ld [hl], d          ; M2: TAC $05 (enable, tap bit 1)
    ld [bc], a          ; M4: TMA $FF
    ld [hl], e          ; M6: TAC $00
    ldh a, [$FF05]      ; M9: TIMA
    finish $00
