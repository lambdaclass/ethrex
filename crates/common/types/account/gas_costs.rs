/// Contains the static gas costs of the EVM instructions (EVM specification constants).

// Opcodes cost
pub const STOP: u64 = 0;
pub const ADD: u64 = 3;
pub const MUL: u64 = 5;
pub const SUB: u64 = 3;
pub const DIV: u64 = 5;
pub const SDIV: u64 = 5;
pub const MOD: u64 = 5;
pub const SMOD: u64 = 5;
pub const ADDMOD: u64 = 8;
pub const MULMOD: u64 = 8;
pub const EXP_STATIC: u64 = 10;
pub const EXP_DYNAMIC_BASE: u64 = 50;
pub const SIGNEXTEND: u64 = 5;
pub const LT: u64 = 3;
pub const GT: u64 = 3;
pub const SLT: u64 = 3;
pub const SGT: u64 = 3;
pub const EQ: u64 = 3;
pub const ISZERO: u64 = 3;
pub const AND: u64 = 3;
pub const OR: u64 = 3;
pub const XOR: u64 = 3;
pub const NOT: u64 = 3;
pub const BYTE: u64 = 3;
pub const SHL: u64 = 3;
pub const SHR: u64 = 3;
pub const SAR: u64 = 3;
pub const KECCAK25_STATIC: u64 = 30;
pub const KECCAK25_DYNAMIC_BASE: u64 = 6;
pub const CALLDATALOAD: u64 = 3;
pub const CALLDATASIZE: u64 = 2;
pub const CALLDATACOPY_STATIC: u64 = 3;
pub const CALLDATACOPY_DYNAMIC_BASE: u64 = 3;
pub const RETURNDATASIZE: u64 = 2;
pub const RETURNDATACOPY_STATIC: u64 = 3;
pub const RETURNDATACOPY_DYNAMIC_BASE: u64 = 3;
pub const ADDRESS: u64 = 2;
pub const ORIGIN: u64 = 2;
pub const CALLER: u64 = 2;
pub const BLOCKHASH: u64 = 20;
pub const COINBASE: u64 = 2;
pub const TIMESTAMP: u64 = 2;
pub const NUMBER: u64 = 2;
pub const PREVRANDAO: u64 = 2;
pub const GASLIMIT: u64 = 2;
pub const CHAINID: u64 = 2;
pub const SELFBALANCE: u64 = 5;
pub const BASEFEE: u64 = 2;
pub const BLOBHASH: u64 = 3;
pub const BLOBBASEFEE: u64 = 2;
pub const SLOTNUM: u64 = 2;
pub const POP: u64 = 2;
pub const MLOAD_STATIC: u64 = 3;
pub const MSTORE_STATIC: u64 = 3;
pub const MSTORE8_STATIC: u64 = 3;
pub const JUMP: u64 = 8;
pub const JUMPI: u64 = 10;
pub const PC: u64 = 2;
pub const MSIZE: u64 = 2;
pub const GAS: u64 = 2;
pub const JUMPDEST: u64 = 1;
pub const TLOAD: u64 = 100;
pub const TSTORE: u64 = 100;
pub const MCOPY_STATIC: u64 = 3;
pub const MCOPY_DYNAMIC_BASE: u64 = 3;
pub const PUSH0: u64 = 2;
pub const PUSHN: u64 = 3;
pub const DUPN: u64 = 3;
pub const SWAPN: u64 = 3;
pub const EXCHANGE: u64 = 3;
pub const LOGN_STATIC: u64 = 375;
pub const LOGN_DYNAMIC_BASE: u64 = 375;
pub const LOGN_DYNAMIC_BYTE_BASE: u64 = 8;
pub const CALLVALUE: u64 = 2;
pub const CODESIZE: u64 = 2;
pub const CODECOPY_STATIC: u64 = 3;
pub const CODECOPY_DYNAMIC_BASE: u64 = 3;
pub const GASPRICE: u64 = 2;
pub const CLZ: u64 = 5;

pub const SELFDESTRUCT_STATIC: u64 = 5000;
pub const SELFDESTRUCT_DYNAMIC: u64 = 25000;
pub const SELFDESTRUCT_REFUND: u64 = 24000;

pub const DEFAULT_STATIC: u64 = 0;
pub const DEFAULT_COLD_DYNAMIC: u64 = 2600;
pub const DEFAULT_WARM_DYNAMIC: u64 = 100;

pub const SLOAD_STATIC: u64 = 0;
pub const SLOAD_COLD_DYNAMIC: u64 = 2100;
pub const SLOAD_WARM_DYNAMIC: u64 = 100;

pub const SSTORE_STATIC: u64 = 0;
pub const SSTORE_COLD_DYNAMIC: u64 = 2100;
pub const SSTORE_DEFAULT_DYNAMIC: u64 = 100;
pub const SSTORE_STORAGE_CREATION: u64 = 20000;
pub const SSTORE_STORAGE_MODIFICATION: u64 = 2900;
pub const SSTORE_STIPEND: i64 = 2300;

pub const BALANCE_STATIC: u64 = DEFAULT_STATIC;
pub const BALANCE_COLD_DYNAMIC: u64 = DEFAULT_COLD_DYNAMIC;
pub const BALANCE_WARM_DYNAMIC: u64 = DEFAULT_WARM_DYNAMIC;

pub const EXTCODESIZE_STATIC: u64 = DEFAULT_STATIC;
pub const EXTCODESIZE_COLD_DYNAMIC: u64 = DEFAULT_COLD_DYNAMIC;
pub const EXTCODESIZE_WARM_DYNAMIC: u64 = DEFAULT_WARM_DYNAMIC;

pub const EXTCODEHASH_STATIC: u64 = DEFAULT_STATIC;
pub const EXTCODEHASH_COLD_DYNAMIC: u64 = DEFAULT_COLD_DYNAMIC;
pub const EXTCODEHASH_WARM_DYNAMIC: u64 = DEFAULT_WARM_DYNAMIC;

pub const EXTCODECOPY_STATIC: u64 = 0;
pub const EXTCODECOPY_DYNAMIC_BASE: u64 = 3;
pub const EXTCODECOPY_COLD_DYNAMIC: u64 = DEFAULT_COLD_DYNAMIC;
pub const EXTCODECOPY_WARM_DYNAMIC: u64 = DEFAULT_WARM_DYNAMIC;

pub const CALL_STATIC: u64 = DEFAULT_STATIC;
pub const CALL_COLD_DYNAMIC: u64 = DEFAULT_COLD_DYNAMIC;
pub const CALL_WARM_DYNAMIC: u64 = DEFAULT_WARM_DYNAMIC;
pub const CALL_POSITIVE_VALUE: u64 = 9000;
pub const CALL_POSITIVE_VALUE_STIPEND: u64 = 2300;
pub const CALL_TO_EMPTY_ACCOUNT: u64 = 25000;

pub const CALLCODE_STATIC: u64 = DEFAULT_STATIC;
pub const CALLCODE_COLD_DYNAMIC: u64 = DEFAULT_COLD_DYNAMIC;
pub const CALLCODE_WARM_DYNAMIC: u64 = DEFAULT_WARM_DYNAMIC;
pub const CALLCODE_POSITIVE_VALUE: u64 = 9000;
pub const CALLCODE_POSITIVE_VALUE_STIPEND: u64 = 2300;

pub const DELEGATECALL_STATIC: u64 = DEFAULT_STATIC;
pub const DELEGATECALL_COLD_DYNAMIC: u64 = DEFAULT_COLD_DYNAMIC;
pub const DELEGATECALL_WARM_DYNAMIC: u64 = DEFAULT_WARM_DYNAMIC;

pub const STATICCALL_STATIC: u64 = DEFAULT_STATIC;
pub const STATICCALL_COLD_DYNAMIC: u64 = DEFAULT_COLD_DYNAMIC;
pub const STATICCALL_WARM_DYNAMIC: u64 = DEFAULT_WARM_DYNAMIC;

// Costs in gas for call opcodes
pub const WARM_ADDRESS_ACCESS_COST: u64 = 100;
pub const COLD_ADDRESS_ACCESS_COST: u64 = 2600;
pub const NON_ZERO_VALUE_COST: u64 = 9000;

pub const VALUE_TO_EMPTY_ACCOUNT_COST: u64 = 25000;

// Costs in gas for create opcodes
pub const INIT_CODE_WORD_COST: u64 = 2;
pub const CODE_DEPOSIT_COST: u64 = 200;
pub const CREATE_BASE_COST: u64 = 32000;

// Calldata costs
pub const CALLDATA_COST_ZERO_BYTE: u64 = 4;
pub const CALLDATA_COST_NON_ZERO_BYTE: u64 = 16;
pub const STANDARD_TOKEN_COST: u64 = 4;

// Blob gas costs
pub const BLOB_GAS_PER_BLOB: u64 = 131072;

// Access lists costs
pub const ACCESS_LIST_STORAGE_KEY_COST: u64 = 1900;
pub const ACCESS_LIST_ADDRESS_COST: u64 = 2400;

// Precompile costs
pub const ECRECOVER_COST: u64 = 3000;
pub const BLS12_381_G1ADD_COST: u64 = 375;
pub const BLS12_381_G2ADD_COST: u64 = 600;
pub const BLS12_381_MAP_FP_TO_G1_COST: u64 = 5500;
pub const BLS12_PAIRING_CHECK_MUL_COST: u64 = 32600;
pub const BLS12_PAIRING_CHECK_FIXED_COST: u64 = 37700;
pub const BLS12_381_MAP_FP2_TO_G2_COST: u64 = 23800;
pub const P256_VERIFY_COST: u64 = 6900;

// Floor cost per token, specified in https://eips.ethereum.org/EIPS/eip-7623
pub const TOTAL_COST_FLOOR_PER_TOKEN: u64 = 10;

pub const SHA2_256_STATIC_COST: u64 = 60;
pub const SHA2_256_DYNAMIC_BASE: u64 = 12;

pub const RIPEMD_160_STATIC_COST: u64 = 600;
pub const RIPEMD_160_DYNAMIC_BASE: u64 = 120;

pub const IDENTITY_STATIC_COST: u64 = 15;
pub const IDENTITY_DYNAMIC_BASE: u64 = 3;

pub const MODEXP_STATIC_COST: u64 = 200;
pub const MODEXP_DYNAMIC_QUOTIENT: u64 = 3;
pub const MODEXP_EXPONENT_FACTOR: u64 = 8;

pub const MODEXP_STATIC_COST_OSAKA: u64 = 500;
pub const MODEXP_DYNAMIC_QUOTIENT_OSAKA: u64 = 1;
pub const MODEXP_EXPONENT_FACTOR_OSAKA: u64 = 16;

pub const ECADD_COST: u64 = 150;
pub const ECMUL_COST: u64 = 6000;

pub const ECPAIRING_BASE_COST: u64 = 45000;
pub const ECPAIRING_GROUP_COST: u64 = 34000;

pub const POINT_EVALUATION_COST: u64 = 50000;

pub const BLAKE2F_ROUND_COST: u64 = 1;

pub const BLS12_381_MSM_MULTIPLIER: u64 = 1000;
pub const BLS12_381_G1_K_DISCOUNT: [u64; 128] = [
    1000, 949, 848, 797, 764, 750, 738, 728, 719, 712, 705, 698, 692, 687, 682, 677, 673, 669, 665,
    661, 658, 654, 651, 648, 645, 642, 640, 637, 635, 632, 630, 627, 625, 623, 621, 619, 617, 615,
    613, 611, 609, 608, 606, 604, 603, 601, 599, 598, 596, 595, 593, 592, 591, 589, 588, 586, 585,
    584, 582, 581, 580, 579, 577, 576, 575, 574, 573, 572, 570, 569, 568, 567, 566, 565, 564, 563,
    562, 561, 560, 559, 558, 557, 556, 555, 554, 553, 552, 551, 550, 549, 548, 547, 547, 546, 545,
    544, 543, 542, 541, 540, 540, 539, 538, 537, 536, 536, 535, 534, 533, 532, 532, 531, 530, 529,
    528, 528, 527, 526, 525, 525, 524, 523, 522, 522, 521, 520, 520, 519,
];
pub const G1_MUL_COST: u64 = 12000;
pub const BLS12_381_G2_K_DISCOUNT: [u64; 128] = [
    1000, 1000, 923, 884, 855, 832, 812, 796, 782, 770, 759, 749, 740, 732, 724, 717, 711, 704,
    699, 693, 688, 683, 679, 674, 670, 666, 663, 659, 655, 652, 649, 646, 643, 640, 637, 634, 632,
    629, 627, 624, 622, 620, 618, 615, 613, 611, 609, 607, 606, 604, 602, 600, 598, 597, 595, 593,
    592, 590, 589, 587, 586, 584, 583, 582, 580, 579, 578, 576, 575, 574, 573, 571, 570, 569, 568,
    567, 566, 565, 563, 562, 561, 560, 559, 558, 557, 556, 555, 554, 553, 552, 552, 551, 550, 549,
    548, 547, 546, 545, 545, 544, 543, 542, 541, 541, 540, 539, 538, 537, 537, 536, 535, 535, 534,
    533, 532, 532, 531, 530, 530, 529, 528, 528, 527, 526, 526, 525, 524, 524,
];
pub const G2_MUL_COST: u64 = 22500;
