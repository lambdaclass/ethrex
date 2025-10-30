.macro  blake2b_mix0 x
    // G(x)
    vpaddq  ymm0,   ymm0,   ymm1
    vpaddq  ymm0,   ymm0,   \x
    vpxor   ymm3,   ymm3,   ymm0
    vpshufd ymm3,   ymm3,   0xB1
    vpaddq  ymm2,   ymm2,   ymm3
    vpxor   ymm1,   ymm1,   ymm2
    vpshufb ymm1,   ymm1,   ymm14
.endm

.macro  blake2b_mix1 x
    // G(y)
    vpaddq  ymm0,   ymm0,   ymm1
    vpaddq  ymm0,   ymm0,   \x
    vpxor   ymm3,   ymm3,   ymm0
    vpshufb ymm3,   ymm3,   ymm15
    vpaddq  ymm2,   ymm2,   ymm3
    vpxor   ymm1,   ymm1,   ymm2
    vpsrlq  ymm12,  ymm1,   63
    vpsllq  ymm1,   ymm1,   1
    vpor    ymm1,   ymm1,   ymm12
.endm

.macro  blake2b_diag
    vpermq      ymm0,   ymm0,   0x93
    vpermq      ymm2,   ymm2,   0x39
    vperm2i128  ymm3,   ymm3,   ymm3,   0x01
.endm

.macro  blake2b_undiag
    vpermq      ymm0,   ymm0,   0x39
    vpermq      ymm2,   ymm2,   0x93
    vperm2i128  ymm3,   ymm3,   ymm3,   0x01
.endm


    .global _blake2b_f
    .type   _blake2b_f, @function
_blake2b_f:
    # rdi <- r: usize,
    # rsi <- h: &mut [u64; 8],
    # rdx <- m: &[u64; 16],
    # rcx <- t: &[u64; 2],
    # r8  <- f: bool

    vzeroall

    # Allocate space for shuffled message.
    mov     r9,     rsp
    sub     rsp,    0x0500  # Allocate space for 32B * 4 * 10 rounds.
    and     rsp,    -0x20   # Align to 32B boundary.

    # Load required constants.
    vbroadcasti128  ymm14,  [rip + blake2b_ror24]
    vbroadcasti128  ymm15,  [rip + blake2b_ror16]

    #
    # Initialize local work vector.
    #
    lea     rax,    [rip + blake2b_iv]
    add     r8,     0x01
    shl     r8,     0x05
    vmovdqu ymm0,   [rsi + 0x00]
    vmovdqu ymm1,   [rsi + 0x20]
    vmovdqa ymm2,   [rax]
    vmovdqa ymm3,   [rax + r8]

    # Apply block number to local work vector.
    pxor    xmm3,   [rcx]

    # Skip every round if `r == 0`.
    sub     rdi,    0x01
    jc      1f

    #
    # First iteration and message shuffling.
    #
    vbroadcasti128  ymm4,   [rdx + 0x00]
    vbroadcasti128  ymm5,   [rdx + 0x10]
    vbroadcasti128  ymm6,   [rdx + 0x20]
    vbroadcasti128  ymm7,   [rdx + 0x30]
    vbroadcasti128  ymm8,   [rdx + 0x40]
    vbroadcasti128  ymm9,   [rdx + 0x50]
    vbroadcasti128  ymm10,  [rdx + 0x60]
    vbroadcasti128  ymm11,  [rdx + 0x70]

    # Round #0:
    #   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
    #   Into: [0 2 4 6 1 3 5 7 E 8 A C F 9 B D]
    vpunpcklqdq     ymm12,  ymm4,   ymm5
    vpunpcklqdq     ymm13,  ymm6,   ymm7
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x0000], ymm12
    blake2b_mix0    ymm12
    vpunpckhqdq     ymm12,  ymm4,   ymm5
    vpunpckhqdq     ymm13,  ymm6,   ymm7
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x0020], ymm12
    blake2b_mix1    ymm12
    blake2b_diag
    vpunpcklqdq     ymm12,  ymm11,  ymm8
    vpunpcklqdq     ymm13,  ymm9,   ymm10
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x0040], ymm12
    blake2b_mix0    ymm12
    vpunpckhqdq     ymm12,  ymm11,  ymm8
    vpunpckhqdq     ymm13,  ymm9,   ymm10
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x0060], ymm12
    blake2b_mix1    ymm12
    blake2b_undiag

    sub     rdi,    0x01
    jc      1f

    # Round #1:
    #   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
    #   Into: [E 4 9 D A 8 F 6 5 1 0 B 3 C 2 7]
    vpunpcklqdq     ymm12,  ymm11,  ymm6
    vpunpckhqdq     ymm13,  ymm8,   ymm10
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x0080], ymm12
    blake2b_mix0    ymm12
    vpunpcklqdq     ymm12,  ymm9,   ymm8
    vpalignr        ymm13,  ymm7,   ymm11,  0x08
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x00A0], ymm12
    blake2b_mix1    ymm12
    blake2b_diag
    vpunpckhqdq     ymm12,  ymm6,   ymm4
    vpblendd        ymm13,  ymm4,   ymm9,   0xCC
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x00C0], ymm12
    blake2b_mix0    ymm12
    vpalignr        ymm12,  ymm10,  ymm5,   0x08
    vpblendd        ymm13,  ymm5,   ymm7,   0xCC
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x00E0], ymm12
    blake2b_mix1    ymm12
    blake2b_undiag

    sub     rdi,    0x01
    jc      1f

    # Round #2:
    #   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
    #   Into: [B C 5 F 8 0 2 D 9 A 3 7 4 E 6 1]
    vpalignr        ymm12,  ymm10,  ymm9,   0x08
    vpunpckhqdq     ymm13,  ymm6,   ymm11
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x0100], ymm12
    blake2b_mix0    ymm12
    vpunpcklqdq     ymm12,  ymm8,   ymm4
    vpblendd        ymm13,  ymm5,   ymm10,  0xCC
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x0120], ymm12
    blake2b_mix1    ymm12
    blake2b_diag
    vpalignr        ymm12,  ymm9,   ymm8,   0x08
    vpunpckhqdq     ymm13,  ymm5,   ymm7
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x0140], ymm12
    blake2b_mix0    ymm12
    vpunpcklqdq     ymm12,  ymm6,   ymm11
    vpblendd        ymm13,  ymm7,   ymm4,   0xCC
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x0160], ymm12
    blake2b_mix1    ymm12
    blake2b_undiag

    sub     rdi,    0x01
    jc      1f

    # Round #3:
    #   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
    #   Into: [7 3 D B 9 1 C E F 2 5 4 8 6 A 0]
    vpunpckhqdq     ymm12,  ymm7,   ymm5
    vpunpckhqdq     ymm13,  ymm10,  ymm9
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x0180], ymm12
    blake2b_mix0    ymm12
    vpunpckhqdq     ymm12,  ymm8,   ymm4
    vpunpcklqdq     ymm13,  ymm10,  ymm11
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x01A0], ymm12
    blake2b_mix1    ymm12
    blake2b_diag
    vpalignr        ymm12,  ymm5,   ymm11,  0x08
    vpshufd         ymm13,  ymm6,   0x4E
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x01C0], ymm12
    blake2b_mix0    ymm12
    vpunpcklqdq     ymm12,  ymm8,   ymm7
    vpunpcklqdq     ymm13,  ymm9,   ymm4
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x01E0], ymm12
    blake2b_mix1    ymm12
    blake2b_undiag

    sub     rdi,    0x01
    jc      1f

    # Round #4:
    #   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
    #   Into: [9 5 2 A 0 7 4 F 3 E B 6 D 1 C 8]
    vpunpckhqdq     ymm12,  ymm8,   ymm6
    vpunpcklqdq     ymm13,  ymm5,   ymm9
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x0200], ymm12
    blake2b_mix0    ymm12
    vpblendd        ymm12,  ymm4,   ymm7,   0xCC
    vpblendd        ymm13,  ymm6,   ymm11,  0xCC
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x0220], ymm12
    blake2b_mix1    ymm12
    blake2b_diag
    vpalignr        ymm12,  ymm11,  ymm5,   0x08
    vpalignr        ymm13,  ymm7,   ymm9,   0x08
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x0240], ymm12
    blake2b_mix0    ymm12
    vpunpckhqdq     ymm12,  ymm10,  ymm4
    vpunpcklqdq     ymm13,  ymm10,  ymm8
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x0260], ymm12
    blake2b_mix1    ymm12
    blake2b_undiag

    sub     rdi,    0x01
    jc      1f

    # Round #5:
    #   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
    #   Into: [2 6 0 8 C A B 3 1 4 7 F 9 D 5 E]
    vpunpcklqdq     ymm12,  ymm5,   ymm7
    vpunpcklqdq     ymm13,  ymm4,   ymm8
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x0280], ymm12
    blake2b_mix0    ymm12
    vpunpcklqdq     ymm12,  ymm10,  ymm9
    vpunpckhqdq     ymm13,  ymm9,   ymm5
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x02A0], ymm12
    blake2b_mix1    ymm12
    blake2b_diag
    vpalignr        ymm12,  ymm6,   ymm4,   0x08
    vpunpckhqdq     ymm13,  ymm7,   ymm11
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x02C0], ymm12
    blake2b_mix0    ymm12
    vpunpckhqdq     ymm12,  ymm8,   ymm10
    vpalignr        ymm13,  ymm11,  ymm6,   0x08
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x02E0], ymm12
    blake2b_mix1    ymm12
    blake2b_undiag

    sub     rdi,    0x01
    jc      1f

    # Round #6:
    #   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
    #   Into: [C 1 E 4 5 F D A 8 0 6 9 B 7 3 2]
    vpblendd        ymm12,  ymm10,  ymm4,   0xCC
    vpunpcklqdq     ymm13,  ymm11,  ymm6
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x0300], ymm12
    blake2b_mix0    ymm12
    vpunpckhqdq     ymm12,  ymm6,   ymm11
    vpalignr        ymm13,  ymm9,   ymm10,  0x08
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x0320], ymm12
    blake2b_mix1    ymm12
    blake2b_diag
    vpunpcklqdq     ymm12,  ymm8,   ymm4
    vpblendd        ymm13,  ymm7,   ymm8,   0xCC
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x0340], ymm12
    blake2b_mix0    ymm12
    vpunpckhqdq     ymm12,  ymm9,   ymm7
    vpshufd         ymm13,  ymm5,   0x4E
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x0360], ymm12
    blake2b_mix1    ymm12
    blake2b_undiag

    sub     rdi,    0x01
    jc      1f

    # Round #7:
    #   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
    #   Into: [D 7 C 3 B E 1 9 2 5 F 8 A 0 4 6]
    vpunpckhqdq     ymm12,  ymm10,  ymm7
    vpblendd        ymm13,  ymm10,  ymm5,   0xCC
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x0380], ymm12
    blake2b_mix0    ymm12
    vpalignr        ymm12,  ymm11,  ymm9,   0x08
    vpunpckhqdq     ymm13,  ymm4,   ymm8
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x03A0], ymm12
    blake2b_mix1    ymm12
    blake2b_diag
    vpblendd        ymm12,  ymm5,   ymm6,   0xCC
    vpalignr        ymm13,  ymm8,   ymm11,  0x08
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x03C0], ymm12
    blake2b_mix0    ymm12
    vpunpcklqdq     ymm12,  ymm9,   ymm4
    vpunpcklqdq     ymm13,  ymm6,   ymm7
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x03E0], ymm12
    blake2b_mix1    ymm12
    blake2b_undiag

    sub     rdi,    0x01
    jc      1f

    # Round #8:
    #   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
    #   Into: [6 E B 0 F 9 3 8 A C D 1 5 2 7 4]
    vpunpcklqdq     ymm12,  ymm7,   ymm11
    vpalignr        ymm13,  ymm4,   ymm9,   0x08
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x0400], ymm12
    blake2b_mix0    ymm12
    vpunpckhqdq     ymm12,  ymm11,  ymm8
    vpalignr        ymm13,  ymm8,   ymm5,   0x08
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x0420], ymm12
    blake2b_mix1    ymm12
    blake2b_diag
    vpunpcklqdq     ymm12,  ymm9,   ymm10
    vpunpckhqdq     ymm13,  ymm10,  ymm4
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x0440], ymm12
    blake2b_mix0    ymm12
    vpalignr        ymm12,  ymm5,   ymm6,   0x08
    vpalignr        ymm13,  ymm6,   ymm7,   0x08
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x0460], ymm12
    blake2b_mix1    ymm12
    blake2b_undiag

    sub     rdi,    0x01
    jc      1f

    # Round #9:
    #   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
    #   Into: [A 8 7 1 2 4 6 5 D F 9 3 0 B E C]
    vpunpcklqdq     ymm12,  ymm9,   ymm8
    vpunpckhqdq     ymm13,  ymm7,   ymm4
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x0480], ymm12
    blake2b_mix0    ymm12
    vpunpcklqdq     ymm12,  ymm5,   ymm6
    vpblendd        ymm13,  ymm7,   ymm6,   0xCC
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x04A0], ymm12
    blake2b_mix1    ymm12
    blake2b_diag
    vpunpckhqdq     ymm12,  ymm10,  ymm11
    vpunpckhqdq     ymm13,  ymm8,   ymm5
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x04C0], ymm12
    blake2b_mix0    ymm12
    vpblendd        ymm12,  ymm4,   ymm9,   0xCC
    vpunpcklqdq     ymm13,  ymm11,  ymm10
    vpblendd        ymm12,  ymm12,  ymm13,  0xF0
    vmovdqa         [rsp + 0x04E0], ymm12
    blake2b_mix1    ymm12
    blake2b_undiag

    sub     rdi,    0x01
    jc     1f

    # Iteration loop.
  0:
    # Round #0:
    blake2b_mix0    [rsp + 0x0000]
    blake2b_mix1    [rsp + 0x0020]
    blake2b_diag
    blake2b_mix0    [rsp + 0x0040]
    blake2b_mix1    [rsp + 0x0060]
    blake2b_undiag

    sub     rdi,    0x01
    jc      1f

    # Round #1:
    blake2b_mix0    [rsp + 0x0080]
    blake2b_mix1    [rsp + 0x00A0]
    blake2b_diag
    blake2b_mix0    [rsp + 0x00C0]
    blake2b_mix1    [rsp + 0x00E0]
    blake2b_undiag

    sub     rdi,    0x01
    jc      1f

    # Round #2:
    blake2b_mix0    [rsp + 0x0100]
    blake2b_mix1    [rsp + 0x0120]
    blake2b_diag
    blake2b_mix0    [rsp + 0x0140]
    blake2b_mix1    [rsp + 0x0160]
    blake2b_undiag

    sub     rdi,    0x01
    jc      1f

    # Round #3:
    blake2b_mix0    [rsp + 0x0180]
    blake2b_mix1    [rsp + 0x01A0]
    blake2b_diag
    blake2b_mix0    [rsp + 0x01C0]
    blake2b_mix1    [rsp + 0x01E0]
    blake2b_undiag

    sub     rdi,    0x01
    jc      1f

    # Round #4:
    blake2b_mix0    [rsp + 0x0200]
    blake2b_mix1    [rsp + 0x0220]
    blake2b_diag
    blake2b_mix0    [rsp + 0x0240]
    blake2b_mix1    [rsp + 0x0260]
    blake2b_undiag

    sub     rdi,    0x01
    jc      1f

    # Round #5:
    blake2b_mix0    [rsp + 0x0280]
    blake2b_mix1    [rsp + 0x02A0]
    blake2b_diag
    blake2b_mix0    [rsp + 0x02C0]
    blake2b_mix1    [rsp + 0x02E0]
    blake2b_undiag

    sub     rdi,    0x01
    jc      1f

    # Round #6:
    blake2b_mix0    [rsp + 0x0300]
    blake2b_mix1    [rsp + 0x0320]
    blake2b_diag
    blake2b_mix0    [rsp + 0x0340]
    blake2b_mix1    [rsp + 0x0360]
    blake2b_undiag

    sub     rdi,    0x01
    jc      1f

    # Round #7:
    blake2b_mix0    [rsp + 0x0380]
    blake2b_mix1    [rsp + 0x03A0]
    blake2b_diag
    blake2b_mix0    [rsp + 0x03C0]
    blake2b_mix1    [rsp + 0x03E0]
    blake2b_undiag

    sub     rdi,    0x01
    jc      1f

    # Round #8:
    blake2b_mix0    [rsp + 0x0400]
    blake2b_mix1    [rsp + 0x0420]
    blake2b_diag
    blake2b_mix0    [rsp + 0x0440]
    blake2b_mix1    [rsp + 0x0460]
    blake2b_undiag

    sub     rdi,    0x01
    jc      1f

    # Round #9:
    blake2b_mix0    [rsp + 0x0480]
    blake2b_mix1    [rsp + 0x04A0]
    blake2b_diag
    blake2b_mix0    [rsp + 0x04C0]
    blake2b_mix1    [rsp + 0x04E0]
    blake2b_undiag

    sub     rdi,    0x01
    jnc     0b

  1:
    # Merge local work vector.
    vpxor   ymm0,   ymm0,   ymm2
    vpxor   ymm1,   ymm1,   ymm3
    vpxor   ymm0,   ymm0,   [rsi + 0x00]
    vpxor   ymm1,   ymm1,   [rsi + 0x20]
    vmovdqu [rsi + 0x00],   ymm0
    vmovdqu [rsi + 0x20],   ymm1

    # Restore original stack pointer.
    mov     rsp,    r9
    ret


    .pushsection    .rodata

    .align  0x20
    .type   blake2b_iv, @object
    .size   blake2b_iv, 0x60
blake2b_iv:
    .quad   0x6A09E667F3BCC908
    .quad   0xBB67AE8584CAA73B
    .quad   0x3C6EF372FE94F82B
    .quad   0xA54FF53A5F1D36F1
    .quad   0x510E527FADE682D1
    .quad   0x9B05688C2B3E6C1F
    .quad   0x1F83D9ABFB41BD6B
    .quad   0x5BE0CD19137E2179

    # Second half of blake2b_iv with inverted bits (for final block).
    .quad   0x510E527FADE682D1
    .quad   0x9B05688C2B3E6C1F
    .quad   0xE07C265404BE4294
    .quad   0x5BE0CD19137E2179

    .align  0x08
    .type   blake2b_ror24,  @object
    .size   blake2b_ror24,  0x10
blake2b_ror24:
    .byte   0x03,   0x04,   0x05,   0x06,   0x07,   0x00,   0x01,   0x02
    .byte   0x0B,   0x0C,   0x0D,   0x0E,   0x0F,   0x08,   0x09,   0x0A

    .align  0x08
    .type   blake2b_ror16,  @object
    .size   blake2b_ror16,  0x10
blake2b_ror16:
    .byte   0x02,   0x03,   0x04,   0x05,   0x06,   0x07,   0x00,   0x01
    .byte   0x0A,   0x0B,   0x0C,   0x0D,   0x0E,   0x0F,   0x08,   0x09

    .popsection
