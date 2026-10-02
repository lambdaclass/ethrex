//! `validate_bal_code_sizes`: no BAL code change may exceed the fork's maximum deployed
//! code size, since code larger than that can never be deployed in a valid block.

use bytes::Bytes;
use ethrex_common::{
    Address, InvalidBlockError,
    constants::AMSTERDAM_MAX_CODE_SIZE,
    types::block_access_list::{AccountChanges, BlockAccessList, CodeChange},
    validate_bal_code_sizes,
};

const MAX: usize = AMSTERDAM_MAX_CODE_SIZE as usize;

fn account_with_code(address: u64, sizes: &[usize]) -> AccountChanges {
    let changes = sizes
        .iter()
        .enumerate()
        .map(|(i, &size)| CodeChange::new(i as u32 + 1, Bytes::from(vec![0x5b; size])))
        .collect();
    AccountChanges::new(Address::from_low_u64_be(address)).with_code_changes(changes)
}

fn check(accounts: Vec<AccountChanges>) -> Result<(), InvalidBlockError> {
    validate_bal_code_sizes(
        &BlockAccessList::from_accounts(accounts),
        AMSTERDAM_MAX_CODE_SIZE,
    )
}

#[test]
fn test_empty_bal_and_empty_code_are_accepted() {
    check(vec![]).expect("an empty BAL has no code changes");
    check(vec![account_with_code(1, &[0])]).expect("empty code is a valid code change");
}

/// Code exactly at the limit can be deployed, so it must pass.
#[test]
fn test_code_at_the_limit_is_accepted() {
    check(vec![account_with_code(1, &[MAX])]).expect("code at the maximum size is deployable");
}

#[test]
fn test_code_one_byte_over_the_limit_is_rejected() {
    let err = check(vec![account_with_code(1, &[MAX + 1])])
        .expect_err("code over the maximum size cannot be deployed");
    assert!(
        matches!(
            err,
            InvalidBlockError::BlockAccessListCodeTooLarge { size, max, .. }
                if size == MAX + 1 && max == AMSTERDAM_MAX_CODE_SIZE
        ),
        "unexpected error: {err:?}"
    );
}

/// Every account and every change is checked, not only the first.
#[test]
fn test_oversized_change_is_found_in_any_account_or_position() {
    let later_account = vec![
        account_with_code(1, &[16, MAX]),
        account_with_code(2, &[MAX + 1]),
    ];
    let later_change = vec![account_with_code(1, &[16, 32, MAX + 1])];

    for accounts in [later_account, later_change] {
        assert!(matches!(
            check(accounts),
            Err(InvalidBlockError::BlockAccessListCodeTooLarge { .. })
        ));
    }
}
