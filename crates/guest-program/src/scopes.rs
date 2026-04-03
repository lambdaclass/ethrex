/// ZisK AIR-cost profiling scope IDs.
///
/// Each constant identifies a logical phase of block execution.
/// These IDs are used as compile-time const generic parameters in
/// `report_cycles` and map to ZisK profiling scope start/end markers.
///
/// ID range: 30-39. Valid range: 0-255.
pub const PRE_STATE_INIT: u16 = 30;
pub const ANCESTOR_VALIDATION: u16 = 31;
pub const PRE_STATE_VERIFICATION: u16 = 32;
pub const VALIDATE_BLOCK_CONSENSUS: u16 = 33;
pub const SETUP_EVM: u16 = 34;
pub const EXECUTE_BLOCK: u16 = 35;
pub const GET_STATE_TRANSITIONS: u16 = 36;
pub const APPLY_ACCOUNT_UPDATES: u16 = 37;
pub const POST_VALIDATION_CHECKS: u16 = 38;
pub const POST_STATE_ROOT_CALCULATION: u16 = 39;

/// ELF symbols for ziskemu name resolution.
///
/// These statics embed human-readable names in the guest ELF binary
/// so that `ziskemu` can map numeric scope IDs to names in its output.
#[cfg(feature = "zisk-scopes")]
mod scope_symbols {
    #[unsafe(export_name = "__ZISKOS_PROFILE_ID_30_PRE_STATE_INIT")]
    #[used]
    static _S30: u16 = 30;
    #[unsafe(export_name = "__ZISKOS_PROFILE_ID_31_ANCESTOR_VALIDATION")]
    #[used]
    static _S31: u16 = 31;
    #[unsafe(export_name = "__ZISKOS_PROFILE_ID_32_PRE_STATE_VERIFICATION")]
    #[used]
    static _S32: u16 = 32;
    #[unsafe(export_name = "__ZISKOS_PROFILE_ID_33_VALIDATE_BLOCK_CONSENSUS")]
    #[used]
    static _S33: u16 = 33;
    #[unsafe(export_name = "__ZISKOS_PROFILE_ID_34_SETUP_EVM")]
    #[used]
    static _S34: u16 = 34;
    #[unsafe(export_name = "__ZISKOS_PROFILE_ID_35_EXECUTE_BLOCK")]
    #[used]
    static _S35: u16 = 35;
    #[unsafe(export_name = "__ZISKOS_PROFILE_ID_36_GET_STATE_TRANSITIONS")]
    #[used]
    static _S36: u16 = 36;
    #[unsafe(export_name = "__ZISKOS_PROFILE_ID_37_APPLY_ACCOUNT_UPDATES")]
    #[used]
    static _S37: u16 = 37;
    #[unsafe(export_name = "__ZISKOS_PROFILE_ID_38_POST_VALIDATION_CHECKS")]
    #[used]
    static _S38: u16 = 38;
    #[unsafe(export_name = "__ZISKOS_PROFILE_ID_39_POST_STATE_ROOT_CALCULATION")]
    #[used]
    static _S39: u16 = 39;
}
