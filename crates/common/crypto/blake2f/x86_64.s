    .macro blake2b_mix_inner x, y
        vpaddq  ymm0,   ymm0,   ymm1
        vpaddq  ymm0,   ymm0,   \x
        vpxor   ymm3,   ymm3,   ymm0
        vpshufd ymm3,   ymm3,   0xB1
        vpaddq  ymm2,   ymm2,   ymm3
        vpxor   ymm1,   ymm1,   ymm2
        vpshufb ymm1,   ymm1,   ymm14
        vpaddq  ymm0,   ymm0,   \y
        vpaddq  ymm0,   ymm0,   ymm1
        vpxor   ymm3,   ymm3,   ymm0
        vpshufb ymm3,   ymm3,   ymm15
        vpaddq  ymm2,   ymm2,   ymm3
        vpxor   ymm1,   ymm1,   ymm2
        vpsrlq  ymm13,  ymm1,   63
        vpsllq  ymm1,   ymm1,   1
        vpor    ymm1,   ymm1,   ymm13
    .endm

    .macro blake2b_mix a, b, c, d
        blake2b_mix_inner   \a,     \b
        vpermq      ymm1,   ymm1,   0x39
        vpermq      ymm3,   ymm3,   0x93
        vperm2i128  ymm2,   ymm2,   ymm2,   0x01
        blake2b_mix_inner   \c,     \d
        vpermq      ymm1,   ymm1,   0x93
        vpermq      ymm3,   ymm3,   0x39
        vperm2i128  ymm2,   ymm2,   ymm2,   0x01
    .endm


    .global blake2b_f
    .type   blake2b_f,  @function
blake2b_f:
    # rdi <- h: &mut [u64; 8]
    # rsi <- m: &[u64; 16]
    # rdx <- t: &[u64; 2]
    # rcx <- r: usize
    # r8  <- f: bool

    vbroadcasti128  ymm14,  [rip + blake2b_ror24]
    vbroadcasti128  ymm15,  [rip + blake2b_ror16]

    # Initialize local work vector.
    lea     rax,    [rip + blake2b_iv]
    add     r8,     1
    shl     r8,     5
    vmovdqu ymm0,   [rdi + 0x00]
    vmovdqu ymm1,   [rdi + 0x20]
    vmovdqa ymm2,   [rax]
    vmovdqa ymm3,   [rax + r8]

    # Apply block counter to local work vector.
    pxor    xmm3,   [rdx]

    cmp     rcx,    0
    jz      1f
  0:
    # Round #0:
    #   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
    #   Into: [0 2 4 6 1 3 5 7 E 8 A C F 9 B D]
    vmovdqu     ymm5,   [rsi + 0x00]
    vmovdqu     ymm7,   [rsi + 0x40]
    vpunpcklqdq ymm4,   ymm5,   [rsi + 0x20]
    vpunpckhqdq ymm5,   ymm5,   [rsi + 0x20]
    vpunpcklqdq ymm6,   ymm7,   [rsi + 0x60]
    vpunpckhqdq ymm7,   ymm7,   [rsi + 0x60]
    vpermq      ymm4,   ymm4,   0xD8
    vpermq      ymm5,   ymm5,   0xD8
    vpermq      ymm6,   ymm6,   0xD8
    vpermq      ymm7,   ymm7,   0xD8
    blake2b_mix ymm4,   ymm5,   ymm6,   ymm7
    sub         rcx,    1
    jz          1f

    # Round #1:
    #   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
    #   Into: [E 4 9 D A 8 F 6 5 1 0 B 3 C 2 7]
    vmovq       xmm4,   [rsi + 0x70]
    vmovq       xmm5,   [rsi + 0x50]
    vmovq       xmm6,   [rsi + 0x08]
    vmovq       xmm7,   [rsi + 0x60]
    vmovq       xmm8,   [rsi + 0x48]
    vmovq       xmm9,   [rsi + 0x78]
    vmovq       xmm10,  [rsi + 0x58]
    vmovq       xmm11,  [rsi + 0x38]
    vpinsrq     xmm4,   xmm4,   [rsi + 0x20],   1
    vpinsrq     xmm5,   xmm5,   [rsi + 0x40],   1
    vpinsrq     xmm6,   xmm6,   [rsi + 0x00],   1
    vpinsrq     xmm7,   xmm7,   [rsi + 0x10],   1
    vpinsrq     xmm8,   xmm8,   [rsi + 0x68],   1
    vpinsrq     xmm9,   xmm9,   [rsi + 0x30],   1
    vpinsrq     xmm10,  xmm10,  [rsi + 0x28],   1
    vpinsrq     xmm11,  xmm11,  [rsi + 0x18],   1
    vinserti128 ymm4,   ymm4,   xmm8,   1
    vinserti128 ymm5,   ymm5,   xmm9,   1
    vinserti128 ymm6,   ymm6,   xmm10,  1
    vinserti128 ymm7,   ymm7,   xmm11,  1
    blake2b_mix ymm4,   ymm5,   ymm6,   ymm7
    sub         rcx,    1
    jz          1f

    # Round #2:
    #   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
    #   Into: [B C 5 F 8 0 2 D 9 A 3 7 4 E 6 1]
    vmovq       xmm4,   [rsi + 0x58]
    vmovq       xmm5,   [rsi + 0x40]
    vmovq       xmm6,   [rsi + 0x50]
    vmovq       xmm7,   [rsi + 0x70]
    vmovq       xmm8,   [rsi + 0x28]
    vmovq       xmm9,   [rsi + 0x10]
    vmovq       xmm10,  [rsi + 0x38]
    vmovq       xmm11,  [rsi + 0x08]
    vpinsrq     xmm4,   xmm4,   [rsi + 0x60],   1
    vpinsrq     xmm5,   xmm5,   [rsi + 0x00],   1
    vpinsrq     xmm6,   xmm6,   [rsi + 0x18],   1
    vpinsrq     xmm7,   xmm7,   [rsi + 0x30],   1
    vpinsrq     xmm8,   xmm8,   [rsi + 0x78],   1
    vpinsrq     xmm9,   xmm9,   [rsi + 0x68],   1
    vpinsrq     xmm10,  xmm10,  [rsi + 0x48],   1
    vpinsrq     xmm11,  xmm11,  [rsi + 0x20],   1
    vinserti128 ymm4,   ymm4,   xmm8,   1
    vinserti128 ymm5,   ymm5,   xmm9,   1
    vinserti128 ymm6,   ymm6,   xmm10,  1
    vinserti128 ymm7,   ymm7,   xmm11,  1
    blake2b_mix ymm4,   ymm5,   ymm6,   ymm7
    sub         rcx,    1
    jz          1f

    # Round #3:
    #   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
    #   Into: [7 3 D B 9 1 C E F 2 5 4 8 6 A 0]
    vmovq       xmm4,   [rsi + 0x38]
    vmovq       xmm5,   [rsi + 0x48]
    vmovq       xmm6,   [rsi + 0x10]
    vmovq       xmm7,   [rsi + 0x30]
    vmovq       xmm8,   [rsi + 0x68]
    vmovq       xmm9,   [rsi + 0x60]
    vmovq       xmm10,  [rsi + 0x20]
    vmovq       xmm11,  [rsi + 0x00]
    vpinsrq     xmm4,   xmm4,   [rsi + 0x18],   1
    vpinsrq     xmm5,   xmm5,   [rsi + 0x08],   1
    vpinsrq     xmm6,   xmm6,   [rsi + 0x28],   1
    vpinsrq     xmm7,   xmm7,   [rsi + 0x50],   1
    vpinsrq     xmm8,   xmm8,   [rsi + 0x58],   1
    vpinsrq     xmm9,   xmm9,   [rsi + 0x70],   1
    vpinsrq     xmm10,  xmm10,  [rsi + 0x78],   1
    vpinsrq     xmm11,  xmm11,  [rsi + 0x40],   1
    vinserti128 ymm4,   ymm4,   xmm8,   1
    vinserti128 ymm5,   ymm5,   xmm9,   1
    vinserti128 ymm6,   ymm6,   xmm10,  1
    vinserti128 ymm7,   ymm7,   xmm11,  1
    blake2b_mix ymm4,   ymm5,   ymm6,   ymm7
    sub         rcx,    1
    jz          1f

    # Round #4:
    #   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
    #   Into: [9 5 2 A 0 7 4 F 3 E B 6 D 1 C 8]
    vmovq       xmm4,   [rsi + 0x48]
    vmovq       xmm5,   [rsi + 0x00]
    vmovq       xmm6,   [rsi + 0x70]
    vmovq       xmm7,   [rsi + 0x08]
    vmovq       xmm8,   [rsi + 0x10]
    vmovq       xmm9,   [rsi + 0x20]
    vmovq       xmm10,  [rsi + 0x30]
    vmovq       xmm11,  [rsi + 0x40]
    vpinsrq     xmm4,   xmm4,   [rsi + 0x28],   1
    vpinsrq     xmm5,   xmm5,   [rsi + 0x38],   1
    vpinsrq     xmm6,   xmm6,   [rsi + 0x58],   1
    vpinsrq     xmm7,   xmm7,   [rsi + 0x60],   1
    vpinsrq     xmm8,   xmm8,   [rsi + 0x50],   1
    vpinsrq     xmm9,   xmm9,   [rsi + 0x78],   1
    vpinsrq     xmm10,  xmm10,  [rsi + 0x18],   1
    vpinsrq     xmm11,  xmm11,  [rsi + 0x68],   1
    vinserti128 ymm4,   ymm4,   xmm8,   1
    vinserti128 ymm5,   ymm5,   xmm9,   1
    vinserti128 ymm6,   ymm6,   xmm10,  1
    vinserti128 ymm7,   ymm7,   xmm11,  1
    blake2b_mix ymm4,   ymm5,   ymm6,   ymm7
    sub         rcx,    1
    jz          1f

    # Round #5:
    #   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
    #   Into: [2 6 0 8 C A B 3 1 4 7 F 9 D 5 E]
    vmovq       xmm4,   [rsi + 0x10]
    vmovq       xmm5,   [rsi + 0x60]
    vmovq       xmm6,   [rsi + 0x20]
    vmovq       xmm7,   [rsi + 0x68]
    vmovq       xmm8,   [rsi + 0x00]
    vmovq       xmm9,   [rsi + 0x58]
    vmovq       xmm10,  [rsi + 0x78]
    vmovq       xmm11,  [rsi + 0x70]
    vpinsrq     xmm4,   xmm4,   [rsi + 0x30],   1
    vpinsrq     xmm5,   xmm5,   [rsi + 0x50],   1
    vpinsrq     xmm6,   xmm6,   [rsi + 0x38],   1
    vpinsrq     xmm7,   xmm7,   [rsi + 0x28],   1
    vpinsrq     xmm8,   xmm8,   [rsi + 0x40],   1
    vpinsrq     xmm9,   xmm9,   [rsi + 0x18],   1
    vpinsrq     xmm10,  xmm10,  [rsi + 0x08],   1
    vpinsrq     xmm11,  xmm11,  [rsi + 0x48],   1
    vinserti128 ymm4,   ymm4,   xmm8,   1
    vinserti128 ymm5,   ymm5,   xmm9,   1
    vinserti128 ymm6,   ymm6,   xmm10,  1
    vinserti128 ymm7,   ymm7,   xmm11,  1
    blake2b_mix ymm4,   ymm5,   ymm6,   ymm7
    sub         rcx,    1
    jz          1f

    # Round #6:
    #   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
    #   Into: [C 1 E 4 5 F D A 8 0 6 9 B 7 3 2]
    vmovq       xmm4,   [rsi + 0x60]
    vmovq       xmm5,   [rsi + 0x28]
    vmovq       xmm6,   [rsi + 0x00]
    vmovq       xmm7,   [rsi + 0x38]
    vmovq       xmm8,   [rsi + 0x70]
    vmovq       xmm9,   [rsi + 0x68]
    vmovq       xmm10,  [rsi + 0x48]
    vmovq       xmm11,  [rsi + 0x10]
    vpinsrq     xmm4,   xmm4,   [rsi + 0x08],   1
    vpinsrq     xmm5,   xmm5,   [rsi + 0x78],   1
    vpinsrq     xmm6,   xmm6,   [rsi + 0x30],   1
    vpinsrq     xmm7,   xmm7,   [rsi + 0x18],   1
    vpinsrq     xmm8,   xmm8,   [rsi + 0x20],   1
    vpinsrq     xmm9,   xmm9,   [rsi + 0x50],   1
    vpinsrq     xmm10,  xmm10,  [rsi + 0x40],   1
    vpinsrq     xmm11,  xmm11,  [rsi + 0x58],   1
    vinserti128 ymm4,   ymm4,   xmm8,   1
    vinserti128 ymm5,   ymm5,   xmm9,   1
    vinserti128 ymm6,   ymm6,   xmm10,  1
    vinserti128 ymm7,   ymm7,   xmm11,  1
    blake2b_mix ymm4,   ymm5,   ymm6,   ymm7
    sub         rcx,    1
    jz          1f

    # Round #7:
    #   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
    #   Into: [D 7 C 3 B E 1 9 2 5 F 8 A 0 4 6]
    vmovq       xmm4,   [rsi + 0x68]
    vmovq       xmm5,   [rsi + 0x58]
    vmovq       xmm6,   [rsi + 0x28]
    vmovq       xmm7,   [rsi + 0x00]
    vmovq       xmm8,   [rsi + 0x60]
    vmovq       xmm9,   [rsi + 0x08]
    vmovq       xmm10,  [rsi + 0x40]
    vmovq       xmm11,  [rsi + 0x30]
    vpinsrq     xmm4,   xmm4,   [rsi + 0x38],   1
    vpinsrq     xmm5,   xmm5,   [rsi + 0x70],   1
    vpinsrq     xmm6,   xmm6,   [rsi + 0x78],   1
    vpinsrq     xmm7,   xmm7,   [rsi + 0x20],   1
    vpinsrq     xmm8,   xmm8,   [rsi + 0x18],   1
    vpinsrq     xmm9,   xmm9,   [rsi + 0x48],   1
    vpinsrq     xmm10,  xmm10,  [rsi + 0x10],   1
    vpinsrq     xmm11,  xmm11,  [rsi + 0x50],   1
    vinserti128 ymm4,   ymm4,   xmm8,   1
    vinserti128 ymm5,   ymm5,   xmm9,   1
    vinserti128 ymm6,   ymm6,   xmm10,  1
    vinserti128 ymm7,   ymm7,   xmm11,  1
    blake2b_mix ymm4,   ymm5,   ymm6,   ymm7
    sub         rcx,    1
    jz          1f

    # Round #8:
    #   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
    #   Into: [6 E B 0 F 9 3 8 A C D 1 5 2 7 4]
    vmovq       xmm4,   [rsi + 0x30]
    vmovq       xmm5,   [rsi + 0x78]
    vmovq       xmm6,   [rsi + 0x60]
    vmovq       xmm8,   [rsi + 0x58]
    vmovq       xmm9,   [rsi + 0x18]
    vmovq       xmm10,  [rsi + 0x08]
    vpermq      ymm7,   [rsi + 0x20], 0x4E
    vpermq      ymm11,  [rsi + 0x00], 0xC6
    vpinsrq     xmm4,   xmm4,   [rsi + 0x70],   1
    vpinsrq     xmm5,   xmm5,   [rsi + 0x48],   1
    vpinsrq     xmm6,   xmm6,   [rsi + 0x68],   1
    vpinsrq     xmm8,   xmm8,   [rsi + 0x00],   1
    vpinsrq     xmm9,   xmm9,   [rsi + 0x40],   1
    vpinsrq     xmm10,  xmm10,  [rsi + 0x50],   1
    vinserti128 ymm4,   ymm4,   xmm8,   1
    vinserti128 ymm5,   ymm5,   xmm9,   1
    vinserti128 ymm6,   ymm6,   xmm10,  1
    vpblendd    ymm7,   ymm7,   ymm11,   0x03
    blake2b_mix ymm4,   ymm5,   ymm6,   ymm7
    sub         rcx,    1
    jz          1f

    # Round #9:
    #   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
    #   Into: [A 8 7 1 2 4 6 5 D F 9 3 0 B E C]
    vmovq       xmm4,   [rsi + 0x50]
    vmovq       xmm6,   [rsi + 0x78]
    vmovq       xmm7,   [rsi + 0x58]
    vmovq       xmm8,   [rsi + 0x38]
    vmovq       xmm10,  [rsi + 0x18]
    vmovq       xmm11,  [rsi + 0x60]
    vpermq      ymm5,   [rsi + 0x20],   0x60
    vpermq      ymm9,   [rsi + 0x00],   0xC6
    vpinsrq     xmm4,   xmm4,   [rsi + 0x40],   1
    vpinsrq     xmm6,   xmm6,   [rsi + 0x48],   1
    vpinsrq     xmm7,   xmm7,   [rsi + 0x70],   1
    vpinsrq     xmm8,   xmm8,   [rsi + 0x08],   1
    vpinsrq     xmm10,  xmm10,  [rsi + 0x68],   1
    vpinsrq     xmm11,  xmm11,  [rsi + 0x00],   1
    vinserti128 ymm4,   ymm4,   xmm8,   1
    vpblendd    ymm5,   ymm5,   ymm9,   0x03
    vinserti128 ymm6,   ymm6,   xmm10,  1
    vinserti128 ymm7,   ymm7,   xmm11,  1
    blake2b_mix ymm4,   ymm5,   ymm6,   ymm7
    sub         rcx,    1
    jnz         0b

  1:
    # Merge local work vector.
    vpxor   ymm0,   ymm0,   ymm2
    vpxor   ymm1,   ymm1,   ymm3
    vpxor   ymm0,   ymm0,   [rdi + 0x00]
    vpxor   ymm1,   ymm1,   [rdi + 0x20]
    vmovdqu [rdi + 0x00],   ymm0
    vmovdqu [rdi + 0x20],   ymm1

    ret


    .pushsection    .rodata

    .align  32
    .type   blake2b_iv,  @object
    .size   blake2b_iv,  0x60
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

    .align  8
    .type   blake2b_ror24,    @object
    .size   blake2b_ror24,    0x10
blake2b_ror24:
    .byte   0x03, 0x04, 0x05, 0x06, 0x07, 0x00, 0x01, 0x02
    .byte   0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x08, 0x09, 0x0A

    .align  8
    .type   blake2b_ror16,    @object
    .size   blake2b_ror16,    0x10
blake2b_ror16:
    .byte   0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x00, 0x01
    .byte   0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x08, 0x09

    .popsection
