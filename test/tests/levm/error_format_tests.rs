use ethrex_levm::errors::ExceptionalHalt;

#[test]
fn stack_underflow_display_is_geth_compatible() {
    let err = ExceptionalHalt::StackUnderflow {
        stack_len: 2,
        required: 3,
    };
    assert_eq!(err.to_string(), "stack underflow (2 <=> 3)");
}

#[test]
fn stack_overflow_display_is_geth_compatible() {
    let err = ExceptionalHalt::StackOverflow {
        stack_len: 1024,
        limit: 1024,
    };
    assert_eq!(err.to_string(), "stack limit reached 1024 (1024)");
}
