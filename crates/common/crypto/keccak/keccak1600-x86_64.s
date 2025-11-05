    .type   __KeccakF1600,  @function
    .align  0x20
__KeccakF1600:
    .cfi_startproc
    endbr64

    mov     rax,    [rdi + 0x3C]
    mov     rbx,    [rdi + 0x44]
    mov     rcx,    [rdi + 0x4C]
    mov     rdx,    [rdi + 0x54]
    mov     rbp,    [rdi + 0x5C]
    jmp     .Loop

    .align  0x20
  .Loop:
    mov     r8,     [rdi - 0x64]
    mov     r9,     [rdi - 0x34]
    mov     r10,    [rdi - 0x04]
    mov     r11,    [rdi + 0x2C]

    xor     rcx,    [rdi - 0x54]
    xor     rdx,    [rdi - 0x4C]
    xor     rax,    r8
    xor     rbx,    [rdi - 0x5C]
    xor     rcx,    [rdi - 0x2C]
    xor     rax,    [rdi - 0x3C]
    mov     r12,    rbp
    xor     rbp,    [rdi - 0x44]

    xor     rcx,    r10
    xor     rax,    [rdi - 0x14]
    xor     rdx,    [rdi - 0x24]
    xor     rbx,    r9
    xor     rbp,    [rdi - 0x1C]

    xor     rcx,    [rdi + 0x24]
    xor     rax,    [rdi + 0x14]
    xor     rdx,    [rdi + 0x04]
    xor     rbx,    [rdi - 0x0C]
    xor     rbp,    [rdi + 0x0C]

    mov     r13,    rcx
    rol     rcx,    0x01
    xor     rcx,    rax
    xor     rdx,    r11

    rol     rax,    0x01
    xor     rax,    rdx
    xor     rbx,    [rdi + 0x1C]

    rol     rdx,    0x01
    xor     rdx,    rbx
    xor     rbp,    [rdi + 0x34]

    rol     rbx,    0x01
    xor     rbx,    rbp

    rol     rbp,    0x01
    xor     rbp,    r13
    xor     r9,     rcx
    xor     r10,    rdx
    rol     r9,     0x2C
    xor     r11,    rbp
    xor     r12,    rax
    rol     r10,    0x2B
    xor     r8,     rbx
    mov     r13,    r9
    rol     r11,    0x15
    or      r9,     r10
    xor     r9,     r8
    rol     r12,    0x0E

    xor     r9,     [r15]
    lea     r15,    [r15 + 0x08]

    mov     r14,    r12
    and     r12,    r11
    mov     [rsi - 0x64],   r9
    xor     r12,    r10
    not     r10
    mov     [rsi - 0x54],   r12

    or      r10,    r11
    mov     r12,    [rdi + 0x4C]
    xor     r10,    r13
    mov     [rsi - 0x5C],   r10

    and     r13,    r8
    mov     r9,     [rdi - 0x1C]
    xor     r13,    r14
    mov     r10,    [rdi - 0x14]
    mov     [rsi - 0x44],   r13

    or      r14,    r8
    mov     r8,     [rdi - 0x4C]
    xor     r14,    r11
    mov     r11,    [rdi + 0x1C]
    mov     [rsi - 0x4C],   r14


    xor     r8,     rbp
    xor     r12,    rdx
    rol     r8,     0x1C
    xor     r11,    rcx
    xor     r9,     rax
    rol     r12,    0x3D
    rol     r11,    0x2D
    xor     r10,    rbx
    rol     r9,     0x14
    mov     r13,    r8
    or      r8,     r12
    rol     r10,    0x03

    xor     r8,     r11
    mov     [rsi - 0x24],   r8

    mov     r14,    r9
    and     r9,     r13
    mov     r8,     [rdi - 0x5C]
    xor     r9,     r12
    not     r12
    mov     [rsi - 0x1C],   r9

    or      r12,    r11
    mov     r9,     [rdi - 0x2C]
    xor     r12,    r10
    mov     [rsi - 0x2C],   r12

    and     r11,    r10
    mov     r12,    [rdi + 0x3C]
    xor     r11,    r14
    mov     [rsi - 0x34],   r11

    or      r14,    r10
    mov     r10,    [rdi + 0x04]
    xor     r14,    r13
    mov     r11,    [rdi + 0x34]
    mov     [rsi - 0x3C],   r14


    xor     r10,    rbp
    xor     r11,    rax
    rol     r10,    0x19
    xor     r9,     rdx
    rol     r11,    0x08
    xor     r12,    rbx
    rol     r9,     0x06
    xor     r8,     rcx
    rol     r12,    0x12
    mov     r13,    r10
    and     r10,    r11
    rol     r8,     0x01

    not     r11
    xor     r10,    r9
    mov     [rsi - 0x0C],   r10

    mov     r14,    r12
    and     r12,    r11
    mov     r10,    [rdi - 0x0C]
    xor     r12,    r13
    mov     [rsi - 0x04],   r12

    or      r13,    r9
    mov     r12,    [rdi + 0x54]
    xor     r13,    r8
    mov     [rsi - 0x14],   r13

    and     r9,     r8
    xor     r9,     r14
    mov     [rsi + 0x0C],   r9

    or      r14,    r8
    mov     r9,     [rdi - 0x3C]
    xor     r14,    r11
    mov     r11,    [rdi + 0x24]
    mov     [rsi + 0x04],   r14


    mov     r8,     [rdi - 0x44]

    xor     r10,    rcx
    xor     r11,    rdx
    rol     r10,    0x0A
    xor     r9,     rbx
    rol     r11,    0x0F
    xor     r12,    rbp
    rol     r9,     0x24
    xor     r8,     rax
    rol     r12,    0x38
    mov     r13,    r10
    or      r10,    r11
    rol     r8,     0x1B

    not     r11
    xor     r10,    r9
    mov     [rsi + 0x1C],   r10

    mov     r14,    r12
    or      r12,    r11
    xor     r12,    r13
    mov     [rsi + 0x24],   r12

    and     r13,    r9
    xor     r13,    r8
    mov     [rsi + 0x14],   r13

    or      r9,     r8
    xor     r9,     r14
    mov     [rsi + 0x34],   r9

    and     r8,     r14
    xor     r8,     r11
    mov     [rsi + 0x2C],   r8


    xor     rdx,    [rdi - 0x54]
    xor     rbp,    [rdi - 0x24]
    rol     rdx,    0x3E
    xor     rcx,    [rdi + 0x44]
    rol     rbp,    0x37
    xor     rax,    [rdi + 0x0C]
    rol     rcx,    0x02
    xor     rbx,    [rdi + 0x14]
    xchg    rdi,    rsi
    rol     rax,    0x27
    rol     rbx,    0x29
    mov     r13,    rdx
    and     rdx,    rbp
    not     rbp
    xor     rdx,    rcx
    mov     [rdi + 0x5C],   rdx

    mov     r14,    rax
    and     rax,    rbp
    xor     rax,    r13
    mov     [rdi + 0x3C],   rax

    or      r13,    rcx
    xor     r13,    rbx
    mov     [rdi + 0x54],   r13

    and     rcx,    rbx
    xor     rcx,    r14
    mov     [rdi + 0x4C],   rcx

    or      rbx,    r14
    xor     rbx,    rbp
    mov     [rdi + 0x44],   rbx

    mov     rbp,    rdx
    mov     rdx,    r13

    test    r15,    0xFF
    jnz     .Loop

    lea     r15,    [r15 - 0xC0]
    .byte	0xF3,   0xC3
    .cfi_endproc
    .size   __KeccakF1600,  . - __KeccakF1600

    .global KeccakF1600
    .type   KeccakF1600,    @function
    .align  0x20
KeccakF1600:
    .cfi_startproc
    endbr64


    push    rbx
    .cfi_adjust_cfa_offset  0x08
    .cfi_offset rbx,    -0x10
    push    rbp
    .cfi_adjust_cfa_offset  0x08
    .cfi_offset rbp,    -0x18
    push    r12
    .cfi_adjust_cfa_offset  0x08
    .cfi_offset r12,    -0x20
    push    r13
    .cfi_adjust_cfa_offset  0x08
    .cfi_offset r13,    -0x28
    push    r14
    .cfi_adjust_cfa_offset  0x08
    .cfi_offset r14,    -0x30
    push    r15
    .cfi_adjust_cfa_offset  0x08
    .cfi_offset r15,    -0x38

    lea     rdi,    [rdi + 0x64]
    sub     rsp,    0xC8
    .cfi_adjust_cfa_offset  0xC8


    not     QWORD PTR [rdi - 0x5C]
    not     QWORD PTR [rdi - 0x54]
    not     QWORD PTR [rdi - 0x24]
    not     QWORD PTR [rdi - 0x04]
    not     QWORD PTR [rdi + 0x24]
    not     QWORD PTR [rdi + 0x3C]

    lea     r15,    [rip + iotas]
    lea     rsi,    [rsp + 0x64]

    call    __KeccakF1600

    not     QWORD PTR [rdi - 0x5C]
    not     QWORD PTR [rdi - 0x54]
    not     QWORD PTR [rdi - 0x24]
    not     QWORD PTR [rdi - 0x04]
    not     QWORD PTR [rdi + 0x24]
    not     QWORD PTR [rdi + 0x3C]
    lea     rdi,    [rdi - 0x64]

    lea     r11,    [rsp + 0xF8]
    .cfi_def_cfa    r11,    0x08
    mov     r15,    [r11 - 0x30]
    mov     r14,    [r11 - 0x28]
    mov     r13,    [r11 - 0x20]
    mov     r12,    [r11 - 0x18]
    mov     rbp,    [r11 - 0x10]
    mov     rbx,    [r11 - 0x08]
    lea     rsp,    [r11]
    .cfi_restore    r12
    .cfi_restore    r13
    .cfi_restore    r14
    .cfi_restore    r15
    .cfi_restore    rbp
    .cfi_restore    rbx
    .byte	0xF3,   0xC3
    .cfi_endproc
    .size   KeccakF1600,    . - KeccakF1600

    .global SHA3_absorb
    .type   SHA3_absorb,    @function
    .align  0x20
SHA3_absorb:
    .cfi_startproc
    endbr64


    push    rbx
    .cfi_adjust_cfa_offset  0x08
    .cfi_offset rbx,    -0x10
    push    rbp
    .cfi_adjust_cfa_offset  0x08
    .cfi_offset rbp,    -0x18
    push    r12
    .cfi_adjust_cfa_offset  0x08
    .cfi_offset r12,    -0x20
    push    r13
    .cfi_adjust_cfa_offset  0x08
    .cfi_offset r13,    -0x28
    push    r14
    .cfi_adjust_cfa_offset  0x08
    .cfi_offset r14,    -0x30
    push    r15
    .cfi_adjust_cfa_offset  0x08
    .cfi_offset r15,    -0x38

    lea     rdi,    [rdi + 0x64]
    sub     rsp,    0xE8
    .cfi_adjust_cfa_offset  0xE8


    mov     r9,     rsi
    lea     rsi,    [rsp + 0x64]

    not     QWORD PTR [rdi - 0x5C]
    not     QWORD PTR [rdi - 0x54]
    not     QWORD PTR [rdi - 0x24]
    not     QWORD PTR [rdi - 0x04]
    not     QWORD PTR [rdi + 0x24]
    not     QWORD PTR [rdi + 0x3C]
    lea     r15,    [rip + iotas]

    mov     [rsi + 0x74],   rcx

  .Loop_absorb:
    cmp     rdx,    rcx
    jc      .Ldone_absorb

    shr     rcx,    0x03
    lea     r8,     [rdi - 0x64]

  .Lblock_absorb:
    mov     rax,    [r9]
    lea     r9,     [r9 + 0x08]
    xor     rax,    [r8]
    lea     r8,     [r8 + 0x08]
    sub     rdx,    0x08
    mov     [r8 - 0x08],    rax
    sub     rcx,    0x01
    jnz     .Lblock_absorb

    mov     [rsi + 0x64],   r9
    mov     [rsi + 0x6C],   rdx
    call    __KeccakF1600
    mov     r9,     [rsi + 0x64]
    mov     rdx,    [rsi + 0x6C]
    mov     rcx,    [rsi + 0x74]
    jmp     .Loop_absorb

    .align  0x20
  .Ldone_absorb:
    mov     rax,    rdx

    not     QWORD PTR [rdi - 0x5C]
    not     QWORD PTR [rdi - 0x54]
    not     QWORD PTR [rdi - 0x24]
    not     QWORD PTR [rdi - 0x04]
    not     QWORD PTR [rdi + 0x24]
    not     QWORD PTR [rdi + 0x3C]

    lea     r11,    [rsp + 0x0118]
    .cfi_def_cfa    r11,    0x08
    mov     r15,    [r11 - 0x30]
    mov     r14,    [r11 - 0x28]
    mov     r13,    [r11 - 0x20]
    mov     r12,    [r11 - 0x18]
    mov     rbp,    [r11 - 0x10]
    mov     rbx,    [r11 - 0x08]
    lea     rsp,    [r11]
    .cfi_restore    r12
    .cfi_restore    r13
    .cfi_restore    r14
    .cfi_restore    r15
    .cfi_restore    rbp
    .cfi_restore    rbx
    .byte   0xF3,   0xC3
    .cfi_endproc
    .size   SHA3_absorb,    . - SHA3_absorb

    .global SHA3_squeeze
    .type   SHA3_squeeze,   @function
    .align  0x20
SHA3_squeeze:
    .cfi_startproc
    endbr64


    push    r12
    .cfi_adjust_cfa_offset  0x08
    .cfi_offset r12,    -0x10
    push    r13
    .cfi_adjust_cfa_offset  0x08
    .cfi_offset r13,    -0x18
    push    r14
    .cfi_adjust_cfa_offset  0x08
    .cfi_offset r14,    -0x20
    sub     rsp,    0x20
    .cfi_adjust_cfa_offset  0x20


    shr     rcx,    0x03
    mov     r8,     rdi
    mov     r12,    rsi
    mov     r13,    rdx
    mov     r14,    rcx
    jmp     .Loop_squeeze

    .align  0x20
  .Loop_squeeze:
    cmp     r13,    0x08
    jb      .Ltail_squeeze

    mov     rax,    [r8]
    lea     r8,     [r8 + 0x08]
    mov     [r12],  rax
    lea     r12,    [r12 + 0x08]
    sub     r13,    0x08
    jz      .Ldone_squeeze

    sub     rcx,    0x01
    jnz     .Loop_squeeze

    mov     rcx,    rdi
    call    KeccakF1600
    mov     r8,     rdi
    mov     rcx,    r14
    jmp     .Loop_squeeze

  .Ltail_squeeze:
    mov     rsi,    r8
    mov     rdi,    r12
    mov     rcx,    r13
    .byte   0xF3,   0xA4

  .Ldone_squeeze:
    mov     r14,    [rsp + 0x20]
    mov     r13,    [rsp + 0x28]
    mov     r12,    [rsp + 0x30]
    add     rsp,    0x38
    .cfi_adjust_cfa_offset  -0x38
    .cfi_restore    r12
    .cfi_restore    r13
    .cfi_restore    r14
    .byte   0xF3,   0xC3
    .cfi_endproc
    .size   SHA3_squeeze,   . - SHA3_squeeze


    .align  0x0100
    .quad   0x00,   0x00,   0x00,   0x00,   0x00,   0x00,   0x00,   0x00
    .type   iotas,  @object
iotas:
    .quad   0x0000000000000001
    .quad   0x0000000000008082
    .quad   0x800000000000808A
    .quad   0x8000000080008000
    .quad   0x000000000000808B
    .quad   0x0000000080000001
    .quad   0x8000000080008081
    .quad   0x8000000000008009
    .quad   0x000000000000008A
    .quad   0x0000000000000088
    .quad   0x0000000080008009
    .quad   0x000000008000000A
    .quad   0x000000008000808B
    .quad   0x800000000000008B
    .quad   0x8000000000008089
    .quad   0x8000000000008003
    .quad   0x8000000000008002
    .quad   0x8000000000000080
    .quad   0x000000000000800A
    .quad   0x800000008000000A
    .quad   0x8000000080008081
    .quad   0x8000000000008080
    .quad   0x0000000080000001
    .quad   0x8000000080008008
    .size   iotas,  . - iotas
    .byte   0x4B,   0x65,   0x63,   0x63,   0x61,   0x6B,   0x2D,   0x31
    .byte   0x36,   0x30,   0x30,   0x20,   0x61,   0x62,   0x73,   0x6F
    .byte   0x72,   0x62,   0x20,   0x61,   0x6E,   0x64,   0x20,   0x73
    .byte   0x71,   0x75,   0x65,   0x65,   0x7A,   0x65,   0x20,   0x66
    .byte   0x6F,   0x72,   0x20,   0x78,   0x38,   0x36,   0x5F,   0x36
    .byte   0x34,   0x2C,   0x20,   0x43,   0x52,   0x59,   0x50,   0x54
    .byte   0x4F,   0x47,   0x41,   0x4D,   0x53,   0x20,   0x62,   0x79
    .byte   0x20,   0x3C,   0x61,   0x70,   0x70,   0x72,   0x6F,   0x40
    .byte   0x6F,   0x70,   0x65,   0x6E,   0x73,   0x73,   0x6C,   0x2E
    .byte   0x6F,   0x72,   0x67,   0x3E,   0x00

    .section    .note.gnu.property, "a",    @note
    .long   4,      2f-1f,  5
    .byte   0x47,  0x4E,   0x55,   0x00
  1:
    .long   0xC0000002,    0x04,   0x03
    .align  0x08
  2:
