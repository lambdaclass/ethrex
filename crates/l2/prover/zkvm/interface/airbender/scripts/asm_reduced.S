/*
    Entry point of all programs (_start).

    It initializes DWARF call frame information, the stack pointer, the
    frame pointer (needed for closures to work in start_rust) and the global
    pointer. Then it calls _start_rust.
*/

.section .init, "ax"
.global _start

_start:
    /* Jump to the absolute address defined by the linker script. */
    // for 32bit
    # lui ra, %hi(_abs_start)
    # jr %lo(_abs_start)(ra)

    la ra, _abs_start
    jr ra

_abs_start:
    .cfi_startproc
    .cfi_undefined ra

    .option push
    .option norelax
    la gp, __global_pointer$
    .option pop

    // Assume single core, and put SP to the very top address of the stack region
    la sp, _sstack

    // Set frame pointer
    add s0, sp, zero

    jal zero, _start_rust

    .cfi_endproc

/*
    Machine trap entry point (_machine_start_trap)
*/
.section .trap, "ax"
.global machine_default_start_trap
.align 4
machine_default_start_trap:
    // We assume that exception stack is always saved to MSCRATCH

    // so we swap it with x31
	csrrw x31, mscratch, x31

    // write to exception stack
    # sw x31, -4(sp)
    sw x30, -8(x31)
    sw x29, -12(x31)
    sw x28, -16(x31)
    sw x27, -20(x31)
    sw x26, -24(x31)
    sw x25, -28(x31)
    sw x24, -32(x31)
    sw x23, -36(x31)
    sw x22, -40(x31)
    sw x21, -44(x31)
    sw x20, -48(x31)
    sw x19, -52(x31)
    sw x18, -56(x31)
    sw x17, -60(x31)
    sw x16, -64(x31)
    sw x15, -68(x31)
    sw x14, -72(x31)
    sw x13, -76(x31)
    sw x12, -80(x31)
    sw x11, -84(x31)
    sw x10, -88(x31)
    sw x9, -92(x31)
    sw x8, -96(x31)
    sw x7, -100(x31)
    sw x6, -104(x31)
    sw x5, -108(x31)
    sw x4, -112(x31)
    sw x3, -116(x31)
    sw x2, -120(x31)
    sw x1, -124(x31)

    // we will not restore it, so we are ok to avoid write
    # sw x0, -128(x31)

    // move valid sp into a0,
    mv a0, x31
    csrrw x31, mscratch, x0
    sw x31, -4(a0)
    // restore sp
    mv sp, a0
    // sp is valid now

    addi sp, sp, -128
    // pass pointer as first argument
    add a0, sp, zero

    jal ra, _machine_start_trap_rust

    // set return address into mepc
    csrw mepc, a0

    // save original SP to mscratch for now
    lw a0, 8(sp) // it's original sp that we saved in the stack
    csrw mscratch, a0 // save it for now

    // restore everything we saved

    // it's illegal instruction, so we skip. Anyway can not overwrite x0
    # lw x0, 0(sp)

    lw x1, 4(sp)
    # lw x2, 8(sp) // do not overwrite SP yet
    lw x3, 12(sp)
    lw x4, 16(sp)
    lw x5, 20(sp)
    lw x6, 24(sp)
    lw x7, 28(sp)
    lw x8, 32(sp)
    lw x9, 36(sp)
    lw x10, 40(sp)
    lw x11, 44(sp)
    lw x12, 48(sp)
    lw x13, 52(sp)
    lw x14, 56(sp)
    lw x15, 60(sp)
    lw x16, 64(sp)
    lw x17, 68(sp)
    lw x18, 72(sp)
    lw x19, 76(sp)
    lw x20, 80(sp)
    lw x21, 84(sp)
    lw x22, 88(sp)
    lw x23, 92(sp)
    lw x24, 96(sp)
    lw x25, 100(sp)
    lw x26, 104(sp)
    lw x27, 108(sp)
    lw x28, 112(sp)
    lw x29, 116(sp)
    lw x30, 120(sp)
    lw x31, 124(sp)

    addi sp, sp, 128
    // we popped everything from the stack
    // now save current exception SP to mscratch,
    // and put original SP back
    csrrw	sp, mscratch, sp

    mret

/* Make sure there is an abort when linking */
.section .text.abort
.globl abort
abort:
    j abort
