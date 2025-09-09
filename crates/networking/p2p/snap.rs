use bytes::Bytes;
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::{Store, error::StoreError};

use crate::rlpx::{
    error::RLPxError,
    snap::{
        AccountRange, AccountRangeUnit, AccountStateSlim, ByteCodes, GetAccountRange, GetByteCodes,
        GetStorageRanges, GetTrieNodes, StorageRanges, StorageSlot, TrieNodes,
    },
};

// Request Processing

pub async fn process_account_range_request(
    request: GetAccountRange,
    store: Store,
) -> Result<AccountRange, StoreError> {
    tokio::task::spawn_blocking(move || {
        let mut accounts = vec![];
        let mut bytes_used = 0;
        for (hash, account) in store.iter_accounts(request.root_hash)? {
            if hash >= request.starting_hash {
                let account = AccountStateSlim::from(account);
                bytes_used += 32 + account.length() as u64;
                accounts.push(AccountRangeUnit { hash, account });
            }
            if hash >= request.limit_hash || bytes_used >= request.response_bytes {
                break;
            }
        }
        let proof = proof_to_encodable(store.get_account_range_proof(
            request.root_hash,
            request.starting_hash,
            accounts.last().map(|acc| acc.hash),
        )?);
        Ok(AccountRange {
            id: request.id,
            accounts,
            proof,
        })
    })
    .await
    .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
}

pub async fn process_storage_ranges_request(
    request: GetStorageRanges,
    store: Store,
) -> Result<StorageRanges, StoreError> {
    tokio::task::spawn_blocking(move || {
        let mut slots = vec![];
        let mut proof = vec![];
        let mut bytes_used = 0;

        for hashed_address in request.account_hashes {
            let mut account_slots = vec![];
            let mut res_capped = false;

            if let Some(storage_iter) = store.iter_storage(request.root_hash, hashed_address)? {
                for (hash, data) in storage_iter {
                    if hash >= request.starting_hash {
                        bytes_used += 64_u64; // slot size
                        account_slots.push(StorageSlot { hash, data });
                    }
                    if hash >= request.limit_hash || bytes_used >= request.response_bytes {
                        if bytes_used >= request.response_bytes {
                            res_capped = true;
                        }
                        break;
                    }
                }
            }

            // Generate proofs only if the response doesn't contain the full storage range for the account
            // Aka if the starting hash is not zero or if the response was capped due to byte limit
            if !request.starting_hash.is_zero() || res_capped && !account_slots.is_empty() {
                proof.extend(proof_to_encodable(
                    store
                        .get_storage_range_proof(
                            request.root_hash,
                            hashed_address,
                            request.starting_hash,
                            account_slots.last().map(|acc| acc.hash),
                        )?
                        .unwrap_or_default(),
                ));
            }

            if !account_slots.is_empty() {
                slots.push(account_slots);
            }

            if bytes_used >= request.response_bytes {
                break;
            }
        }
        Ok(StorageRanges {
            id: request.id,
            slots,
            proof,
        })
    })
    .await
    .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
}

pub fn process_byte_codes_request(
    request: GetByteCodes,
    store: Store,
) -> Result<ByteCodes, StoreError> {
    let mut codes = vec![];
    let mut bytes_used = 0;
    for code_hash in request.hashes {
        if let Some(code) = store.get_account_code(code_hash)? {
            bytes_used += code.len() as u64;
            codes.push(code);
        }
        if bytes_used >= request.bytes {
            break;
        }
    }
    Ok(ByteCodes {
        id: request.id,
        codes,
    })
}

pub async fn process_trie_nodes_request(
    request: GetTrieNodes,
    store: Store,
) -> Result<TrieNodes, RLPxError> {
    tokio::task::spawn_blocking(move || {
        let mut nodes = vec![];
        let mut remaining_bytes = request.bytes;
        for paths in request.paths {
            if paths.is_empty() {
                return Err(RLPxError::BadRequest(
                    "zero-item pathset requested".to_string(),
                ));
            }
            let trie_nodes = store.get_trie_nodes(
                request.root_hash,
                paths.into_iter().map(|bytes| bytes.to_vec()).collect(),
                remaining_bytes,
            )?;
            nodes.extend(trie_nodes.iter().map(|nodes| Bytes::copy_from_slice(nodes)));
            remaining_bytes = remaining_bytes
                .saturating_sub(trie_nodes.iter().fold(0, |acc, nodes| acc + nodes.len()) as u64);
            if remaining_bytes == 0 {
                break;
            }
        }

        Ok(TrieNodes {
            id: request.id,
            nodes,
        })
    })
    .await
    .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
}

// Helper method to convert proof to RLP-encodable format
#[inline]
pub(crate) fn proof_to_encodable(proof: Vec<Vec<u8>>) -> Vec<Bytes> {
    proof.into_iter().map(Bytes::from).collect()
}

// Helper method to obtain proof from RLP-encodable format
#[inline]
pub(crate) fn encodable_to_proof(proof: &[Bytes]) -> Vec<Vec<u8>> {
    proof.iter().map(|bytes| bytes.to_vec()).collect()
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use ethrex_common::{BigEndianHash, H256, types::AccountState};
    use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
    use ethrex_storage::EngineType;

    use crate::rlpx::snap::AccountStateSlim;

    use super::*;

    // Hive `AccounRange` Tests
    // Requests & invariantes taken from https://github.com/ethereum/go-ethereum/blob/3e567b8b2901611f004b5a6070a9b6d286be128d/cmd/devp2p/internal/ethtest/snap.go#L69

    use lazy_static::lazy_static;

    lazy_static! {
        // Constant values for hive `AccountRange` tests
        static ref HASH_MIN: H256 = H256::zero();
        static ref HASH_MAX: H256 =
            H256::from_str("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",)
                .unwrap();
        static ref HASH_FIRST: H256 =
            H256::from_str("0x005e94bf632e80cde11add7d3447cd4ca93a5f2205d9874261484ae180718bd6")
                .unwrap();
        static ref HASH_SECOND: H256 =
            H256::from_str("0x00748bacab20da9ae19dd26a33bd10bbf825e28b3de84fc8fe1d15a21645067f")
                .unwrap();
        static ref HASH_FIRST_MINUS_500: H256 = H256::from_uint(&((*HASH_FIRST).into_uint() - 500));
        static ref HASH_FIRST_MINUS_450: H256 = H256::from_uint(&((*HASH_FIRST).into_uint() - 450));
        static ref HASH_FIRST_MINUS_ONE: H256 = H256::from_uint(&((*HASH_FIRST).into_uint() - 1));
        static ref HASH_FIRST_PLUS_ONE: H256 = H256::from_uint(&((*HASH_FIRST).into_uint() + 1));
    }

    #[tokio::test]
    async fn hive_account_range_a() -> Result<(), StoreError> {
        let (store, root) = setup_initial_state()?;
        let request = GetAccountRange {
            id: 0,
            root_hash: root,
            starting_hash: *HASH_MIN,
            limit_hash: *HASH_MAX,
            response_bytes: 4000,
        };
        let res = process_account_range_request(request, store).await.unwrap();
        // Check test invariants
        assert_eq!(res.accounts.len(), 86);
        assert_eq!(res.accounts.first().unwrap().hash, *HASH_FIRST);
        assert_eq!(
            res.accounts.last().unwrap().hash,
            H256::from_str("0x445cb5c1278fdce2f9cbdb681bdd76c52f8e50e41dbd9e220242a69ba99ac099")
                .unwrap()
        );
        Ok(())
    }

    #[tokio::test]
    async fn hive_account_range_b() -> Result<(), StoreError> {
        let (store, root) = setup_initial_state()?;
        let request = GetAccountRange {
            id: 0,
            root_hash: root,
            starting_hash: *HASH_MIN,
            limit_hash: *HASH_MAX,
            response_bytes: 3000,
        };
        let res = process_account_range_request(request, store).await.unwrap();
        // Check test invariants
        assert_eq!(res.accounts.len(), 65);
        assert_eq!(res.accounts.first().unwrap().hash, *HASH_FIRST);
        assert_eq!(
            res.accounts.last().unwrap().hash,
            H256::from_str("0x2e6fe1362b3e388184fd7bf08e99e74170b26361624ffd1c5f646da7067b58b6")
                .unwrap()
        );
        Ok(())
    }

    #[tokio::test]
    async fn hive_account_range_c() -> Result<(), StoreError> {
        let (store, root) = setup_initial_state()?;
        let request = GetAccountRange {
            id: 0,
            root_hash: root,
            starting_hash: *HASH_MIN,
            limit_hash: *HASH_MAX,
            response_bytes: 2000,
        };
        let res = process_account_range_request(request, store).await.unwrap();
        // Check test invariants
        assert_eq!(res.accounts.len(), 44);
        assert_eq!(res.accounts.first().unwrap().hash, *HASH_FIRST);
        assert_eq!(
            res.accounts.last().unwrap().hash,
            H256::from_str("0x1c3f74249a4892081ba0634a819aec9ed25f34c7653f5719b9098487e65ab595")
                .unwrap()
        );
        Ok(())
    }

    #[tokio::test]
    async fn hive_account_range_d() -> Result<(), StoreError> {
        let (store, root) = setup_initial_state()?;
        let request = GetAccountRange {
            id: 0,
            root_hash: root,
            starting_hash: *HASH_MIN,
            limit_hash: *HASH_MAX,
            response_bytes: 1,
        };
        let res = process_account_range_request(request, store).await.unwrap();
        // Check test invariants
        assert_eq!(res.accounts.len(), 1);
        assert_eq!(res.accounts.first().unwrap().hash, *HASH_FIRST);
        assert_eq!(res.accounts.last().unwrap().hash, *HASH_FIRST);
        Ok(())
    }

    #[tokio::test]
    async fn hive_account_range_e() -> Result<(), StoreError> {
        let (store, root) = setup_initial_state()?;
        let request = GetAccountRange {
            id: 0,
            root_hash: root,
            starting_hash: *HASH_MIN,
            limit_hash: *HASH_MAX,
            response_bytes: 0,
        };
        let res = process_account_range_request(request, store).await.unwrap();
        // Check test invariants
        assert_eq!(res.accounts.len(), 1);
        assert_eq!(res.accounts.first().unwrap().hash, *HASH_FIRST);
        assert_eq!(res.accounts.last().unwrap().hash, *HASH_FIRST);
        Ok(())
    }

    #[tokio::test]
    async fn hive_account_range_f() -> Result<(), StoreError> {
        // In this test, we request a range where startingHash is before the first available
        // account key, and limitHash is after. The server should return the first and second
        // account of the state (because the second account is the 'next available').
        let (store, root) = setup_initial_state()?;
        let request = GetAccountRange {
            id: 0,
            root_hash: root,
            starting_hash: *HASH_FIRST_MINUS_500,
            limit_hash: *HASH_FIRST_PLUS_ONE,
            response_bytes: 4000,
        };
        let res = process_account_range_request(request, store).await.unwrap();
        // Check test invariants
        assert_eq!(res.accounts.len(), 2);
        assert_eq!(res.accounts.first().unwrap().hash, *HASH_FIRST);
        assert_eq!(res.accounts.last().unwrap().hash, *HASH_SECOND);
        Ok(())
    }

    #[tokio::test]
    async fn hive_account_range_g() -> Result<(), StoreError> {
        // Here we request range where both bounds are before the first available account key.
        // This should return the first account (even though it's out of bounds).
        let (store, root) = setup_initial_state()?;
        let request = GetAccountRange {
            id: 0,
            root_hash: root,
            starting_hash: *HASH_FIRST_MINUS_500,
            limit_hash: *HASH_FIRST_MINUS_450,
            response_bytes: 4000,
        };
        let res = process_account_range_request(request, store).await.unwrap();
        // Check test invariants
        assert_eq!(res.accounts.len(), 1);
        assert_eq!(res.accounts.first().unwrap().hash, *HASH_FIRST);
        assert_eq!(res.accounts.last().unwrap().hash, *HASH_FIRST);
        Ok(())
    }

    #[tokio::test]
    async fn hive_account_range_h() -> Result<(), StoreError> {
        // In this test, both startingHash and limitHash are zero.
        // The server should return the first available account.
        let (store, root) = setup_initial_state()?;
        let request = GetAccountRange {
            id: 0,
            root_hash: root,
            starting_hash: *HASH_MIN,
            limit_hash: *HASH_MIN,
            response_bytes: 4000,
        };
        let res = process_account_range_request(request, store).await.unwrap();
        // Check test invariants
        assert_eq!(res.accounts.len(), 1);
        assert_eq!(res.accounts.first().unwrap().hash, *HASH_FIRST);
        assert_eq!(res.accounts.last().unwrap().hash, *HASH_FIRST);
        Ok(())
    }

    #[tokio::test]
    async fn hive_account_range_i() -> Result<(), StoreError> {
        let (store, root) = setup_initial_state()?;
        let request = GetAccountRange {
            id: 0,
            root_hash: root,
            starting_hash: *HASH_FIRST,
            limit_hash: *HASH_MAX,
            response_bytes: 4000,
        };
        let res = process_account_range_request(request, store).await.unwrap();
        // Check test invariants
        assert_eq!(res.accounts.len(), 86);
        assert_eq!(res.accounts.first().unwrap().hash, *HASH_FIRST);
        assert_eq!(
            res.accounts.last().unwrap().hash,
            H256::from_str("0x445cb5c1278fdce2f9cbdb681bdd76c52f8e50e41dbd9e220242a69ba99ac099")
                .unwrap()
        );
        Ok(())
    }

    #[tokio::test]
    async fn hive_account_range_j() -> Result<(), StoreError> {
        let (store, root) = setup_initial_state()?;
        let request = GetAccountRange {
            id: 0,
            root_hash: root,
            starting_hash: *HASH_FIRST_PLUS_ONE,
            limit_hash: *HASH_MAX,
            response_bytes: 4000,
        };
        let res = process_account_range_request(request, store).await.unwrap();
        // Check test invariants
        assert_eq!(res.accounts.len(), 86);
        assert_eq!(res.accounts.first().unwrap().hash, *HASH_SECOND);
        assert_eq!(
            res.accounts.last().unwrap().hash,
            H256::from_str("0x4615e5f5df5b25349a00ad313c6cd0436b6c08ee5826e33a018661997f85ebaa")
                .unwrap()
        );
        Ok(())
    }

    // Tests for different roots skipped (we don't have other state's data loaded)

    // Non-sensical requests

    #[tokio::test]
    async fn hive_account_range_k() -> Result<(), StoreError> {
        // In this test, the startingHash is the first available key, and limitHash is
        // a key before startingHash (wrong order). The server should return the first available key.
        let (store, root) = setup_initial_state()?;
        let request = GetAccountRange {
            id: 0,
            root_hash: root,
            starting_hash: *HASH_FIRST,
            limit_hash: *HASH_FIRST_MINUS_ONE,
            response_bytes: 4000,
        };
        let res = process_account_range_request(request, store).await.unwrap();
        // Check test invariants
        assert_eq!(res.accounts.len(), 1);
        assert_eq!(res.accounts.first().unwrap().hash, *HASH_FIRST);
        assert_eq!(res.accounts.last().unwrap().hash, *HASH_FIRST);
        Ok(())
    }

    #[tokio::test]
    async fn hive_account_range_m() -> Result<(), StoreError> {
        // In this test, the startingHash is the first available key and limitHash is zero.
        // (wrong order). The server should return the first available key.
        let (store, root) = setup_initial_state()?;
        let request = GetAccountRange {
            id: 0,
            root_hash: root,
            starting_hash: *HASH_FIRST,
            limit_hash: *HASH_MIN,
            response_bytes: 4000,
        };
        let res = process_account_range_request(request, store).await.unwrap();
        // Check test invariants
        assert_eq!(res.accounts.len(), 1);
        assert_eq!(res.accounts.first().unwrap().hash, *HASH_FIRST);
        assert_eq!(res.accounts.last().unwrap().hash, *HASH_FIRST);
        Ok(())
    }

    // Initial state setup for hive snap tests

    fn setup_initial_state() -> Result<(Store, H256), StoreError> {
        // We cannot process the old blocks that hive uses for the devp2p snap tests
        // So I copied the state from a geth execution of the test suite

        // State was trimmed to only the first 100 accounts (as the furthest account used by the tests is account 87)
        // If the full 408 account state is needed check out previous commits the PR that added this code

        let accounts: Vec<(&str, Vec<u8>)> = vec![
            (
                "0x005e94bf632e80cde11add7d3447cd4ca93a5f2205d9874261484ae180718bd6",
                vec![
                    228_u8, 1, 128, 160, 223, 151, 249, 75, 196, 116, 113, 135, 6, 6, 246, 38, 251,
                    122, 11, 66, 238, 210, 212, 95, 204, 132, 220, 18, 0, 206, 98, 247, 131, 29,
                    169, 144, 128,
                ],
            ),
            (
                "0x00748bacab20da9ae19dd26a33bd10bbf825e28b3de84fc8fe1d15a21645067f",
                vec![196, 128, 1, 128, 128],
            ),
            (
                "0x00aa781aff39a8284ef43790e3a511b2caa50803613c5096bc782e8de08fa4c5",
                vec![
                    211, 128, 143, 192, 151, 206, 123, 201, 7, 21, 179, 75, 159, 16, 0, 0, 0, 0,
                    128, 128,
                ],
            ),
            (
                "0x016d92531f4754834b0502de5b0342ceff21cde5bef386a83d2292f4445782c2",
                vec![196, 128, 1, 128, 128],
            ),
            (
                "0x02547b56492bfe767f3d18be2aab96441c449cd945770ef7ef8555acc505b2e4",
                vec![196, 128, 1, 128, 128],
            ),
            (
                "0x025f478d53bf78add6fa3708d9e061d59bfe14b21329b2a4cf1156d4f81b3d2d",
                vec![201, 128, 133, 23, 72, 118, 232, 0, 128, 128],
            ),
            (
                "0x0267c643f67b47cac9efacf6fcf0e4f4e1b273a727ded155db60eb9907939eb6",
                vec![201, 128, 133, 23, 72, 118, 232, 0, 128, 128],
            ),
            (
                "0x0304d8eaccf0b942c468074250cbcb625ec5c4688b6b5d17d2a9bdd8dd565d5a",
                vec![
                    228, 1, 128, 160, 224, 12, 73, 166, 88, 73, 208, 92, 191, 39, 164, 215, 120,
                    138, 104, 188, 123, 96, 19, 174, 51, 65, 29, 64, 188, 137, 40, 47, 192, 100,
                    243, 61, 128,
                ],
            ),
            (
                "0x0463e52cda557221b0b66bd7285b043071df4c2ab146260f4e010970f3a0cccf",
                vec![196, 1, 128, 128, 128],
            ),
            (
                "0x04d9aa4f67f8b24d70a0ffd757e82456d9184113106b7d9e8eb6c3e8a8df27ee",
                vec![201, 128, 133, 23, 72, 118, 232, 0, 128, 128],
            ),
            (
                "0x053df2c3b574026812b154a99b13b626220af85cd01bb1693b1d42591054bce6",
                vec![201, 128, 133, 23, 72, 118, 232, 0, 128, 128],
            ),
            (
                "0x0579e46a5ed8a88504ac7d579b12eb346fbe4fd7e281bdd226b891f8abed4789",
                vec![
                    228, 1, 128, 160, 61, 14, 43, 165, 55, 243, 89, 65, 6, 135, 9, 69, 15, 37, 254,
                    228, 90, 175, 77, 198, 174, 46, 210, 42, 209, 46, 7, 67, 172, 124, 84, 167,
                    128,
                ],
            ),
            (
                "0x05f6de281d8c2b5d98e8e01cd529bd76416b248caf11e0552047c5f1d516aab6",
                vec![196, 128, 1, 128, 128],
            ),
            (
                "0x07b49045c401bcc408f983d91a199c908cdf0d646049b5b83629a70b0117e295",
                vec![
                    228, 1, 128, 160, 134, 154, 203, 146, 159, 89, 28, 84, 203, 133, 132, 42, 81,
                    242, 150, 99, 94, 125, 137, 87, 152, 197, 71, 162, 147, 175, 228, 62, 123, 247,
                    244, 23, 128,
                ],
            ),
            (
                "0x0993fd5b750fe4414f93c7880b89744abb96f7af1171ed5f47026bdf01df1874",
                vec![196, 128, 1, 128, 128],
            ),
            (
                "0x099d5081762b8b265e8ba4cd8e43f08be4715d903a0b1d96b3d9c4e811cbfb33",
                vec![196, 128, 1, 128, 128],
            ),
            (
                "0x09d6e6745d272389182a510994e2b54d14b731fac96b9c9ef434bc1924315371",
                vec![196, 128, 128, 128, 128],
            ),
            (
                "0x0a93a7231976ad485379a3b66c2d8983ba0b2ca87abaf0ca44836b2a06a2b102",
                vec![196, 128, 1, 128, 128],
            ),
            (
                "0x0b564e4a0203cbcec8301709a7449e2e7371910778df64c89f48507390f2d129",
                vec![196, 1, 128, 128, 128],
            ),
            (
                "0x0cd2a7c53c76f228ed3aa7a29644b1915fde9ec22e0433808bf5467d914e7c7a",
                vec![
                    228, 1, 128, 160, 7, 84, 3, 90, 164, 7, 51, 129, 162, 17, 52, 43, 80, 125, 232,
                    231, 117, 201, 124, 150, 16, 150, 230, 226, 39, 93, 240, 191, 203, 179, 160,
                    28, 128,
                ],
            ),
            (
                "0x0e0e4646090b881949ec9991e48dec768ccd1980896aefd0d51fd56fd5689790",
                vec![
                    228, 1, 128, 160, 96, 252, 105, 16, 13, 142, 99, 38, 103, 200, 11, 148, 212,
                    52, 0, 136, 35, 237, 117, 65, 107, 113, 203, 209, 18, 180, 208, 176, 47, 86,
                    48, 39, 128,
                ],
            ),
            (
                "0x0e27113c09de0a0cb0ff268c677aba17d39a3190fe15aec0ff7f54184955cba4",
                vec![201, 128, 133, 23, 72, 118, 232, 0, 128, 128],
            ),
            (
                "0x0e57ffa6cc6cbd96c1400150417dd9b30d958c58f63c36230a90a02b076f78b5",
                vec![196, 128, 1, 128, 128],
            ),
            (
                "0x0f30822f90f33f1d1ba6d1521a00935630d2c81ab12fa03d4a0f4915033134f3",
                vec![
                    228, 1, 128, 160, 128, 120, 243, 37, 157, 129, 153, 183, 202, 57, 213, 30, 53,
                    213, 181, 141, 113, 255, 20, 134, 6, 115, 16, 96, 56, 109, 50, 60, 93, 25, 24,
                    44, 128,
                ],
            ),
            (
                "0x1017b10a7cc3732d729fe1f71ced25e5b7bc73dc62ca61309a8c7e5ac0af2f72",
                vec![196, 1, 128, 128, 128],
            ),
            (
                "0x1098f06082dc467088ecedb143f9464ebb02f19dc10bd7491b03ba68d751ce45",
                vec![196, 1, 128, 128, 128],
            ),
            (
                "0x11eb0304c1baa92e67239f6947cb93e485a7db05e2b477e1167a8960458fa8cc",
                vec![196, 1, 128, 128, 128],
            ),
            (
                "0x12be3bf1f9b1dab5f908ca964115bee3bcff5371f84ede45bc60591b21117c51",
                vec![201, 128, 133, 23, 72, 118, 232, 0, 128, 128],
            ),
            (
                "0x12c1bb3dddf0f06f62d70ed5b7f7db7d89b591b3f23a838062631c4809c37196",
                vec![196, 128, 1, 128, 128],
            ),
            (
                "0x12e394ad62e51261b4b95c431496e46a39055d7ada7dbf243f938b6d79054630",
                vec![196, 1, 128, 128, 128],
            ),
            (
                "0x13cfc46f6bdb7a1c30448d41880d061c3b8d36c55a29f1c0c8d95a8e882b8c25",
                vec![
                    228, 1, 128, 160, 148, 79, 9, 90, 251, 209, 56, 62, 93, 15, 145, 239, 2, 137,
                    93, 57, 143, 79, 118, 253, 182, 216, 106, 223, 71, 101, 242, 91, 220, 48, 79,
                    95, 128,
                ],
            ),
            (
                "0x15293aec87177f6c88f58bc51274ba75f1331f5cb94f0c973b1deab8b3524dfe",
                vec![196, 128, 1, 128, 128],
            ),
            (
                "0x170c927130fe8f1db3ae682c22b57f33f54eb987a7902ec251fe5dba358a2b25",
                vec![196, 128, 1, 128, 128],
            ),
            (
                "0x17350c7adae7f08d7bbb8befcc97234462831638443cd6dfea186cbf5a08b7c7",
                vec![
                    228, 1, 128, 160, 76, 231, 156, 217, 100, 86, 80, 240, 160, 14, 255, 168, 111,
                    111, 234, 115, 60, 236, 234, 158, 162, 105, 100, 130, 143, 242, 92, 240, 87,
                    123, 201, 116, 128,
                ],
            ),
            (
                "0x174f1a19ff1d9ef72d0988653f31074cb59e2cf37cd9d2992c7b0dd3d77d84f9",
                vec![196, 128, 1, 128, 128],
            ),
            (
                "0x17984cc4b4aac0492699d37662b53ec2acf8cbe540c968b817061e4ed27026d0",
                vec![196, 128, 1, 128, 128],
            ),
            (
                "0x181abdd5e212171007e085fdc284a84d42d5bfc160960d881ccb6a10005ff089",
                vec![196, 1, 128, 128, 128],
            ),
            (
                "0x188111c233bf6516bb9da8b5c4c31809a42e8604cd0158d933435cfd8e06e413",
                vec![196, 1, 128, 128, 128],
            ),
            (
                "0x18f4256a59e1b2e01e96ac465e1d14a45d789ce49728f42082289fc25cf32b8d",
                vec![196, 128, 1, 128, 128],
            ),
            (
                "0x1960414a11f8896c7fc4243aba7ed8179b0bc6979b7c25da7557b17f5dee7bf7",
                vec![196, 1, 128, 128, 128],
            ),
            (
                "0x1a28912018f78f7e754df6b9fcec33bea25e5a232224db622e0c3343cf079eff",
                vec![201, 128, 133, 23, 72, 118, 232, 0, 128, 128],
            ),
            (
                "0x1bf7626cec5330a127e439e68e6ee1a1537e73b2de1aa6d6f7e06bc0f1e9d763",
                vec![196, 128, 1, 128, 128],
            ),
            (
                "0x1c248f110218eaae2feb51bc82e9dcc2844bf93b88172c52afcb86383d262323",
                vec![201, 128, 133, 23, 72, 118, 232, 0, 128, 128],
            ),
            (
                "0x1c3f74249a4892081ba0634a819aec9ed25f34c7653f5719b9098487e65ab595",
                vec![
                    228, 1, 128, 160, 175, 134, 126, 108, 186, 232, 16, 202, 169, 36, 184, 182,
                    172, 61, 140, 8, 145, 131, 20, 145, 166, 144, 109, 208, 190, 122, 211, 36, 220,
                    209, 83, 61, 128,
                ],
            ),
            (
                "0x1d38ada74301c31f3fd7d92dd5ce52dc37ae633e82ac29c4ef18dfc141298e26",
                vec![201, 128, 133, 23, 72, 118, 232, 0, 128, 128],
            ),
            (
                "0x1d6ee979097e29141ad6b97ae19bb592420652b7000003c55eb52d5225c3307d",
                vec![
                    228, 1, 128, 160, 247, 53, 145, 231, 145, 175, 76, 124, 95, 160, 57, 195, 61,
                    217, 209, 105, 202, 177, 75, 29, 155, 12, 167, 139, 204, 78, 116, 13, 85, 59,
                    26, 207, 128,
                ],
            ),
            (
                "0x1dff76635b74ddba16bba3054cc568eed2571ea6becaabd0592b980463f157e2",
                vec![196, 1, 128, 128, 128],
            ),
            (
                "0x1ee7e0292fba90d9733f619f976a2655c484adb30135ef0c5153b5a2f32169df",
                vec![196, 1, 128, 128, 128],
            ),
            (
                "0x209b102e507b8dfc6acfe2cf55f4133b9209357af679a6d507e6ee87112bfe10",
                vec![196, 1, 128, 128, 128],
            ),
            (
                "0x210ce6d692a21d75de3764b6c0356c63a51550ebec2c01f56c154c24b1cf8888",
                vec![196, 1, 128, 128, 128],
            ),
            (
                "0x2116ab29b4cb8547af547fe472b7ce30713f234ed49cb1801ea6d3cf9c796d57",
                vec![201, 128, 133, 23, 72, 118, 232, 0, 128, 128],
            ),
            (
                "0x2290ea88cc63f09ab5e8c989a67e2e06613311801e39c84aae3badd8bb38409c",
                vec![201, 128, 133, 23, 72, 118, 232, 0, 128, 128],
            ),
            (
                "0x2369a492b6cddcc0218617a060b40df0e7dda26abe48ba4e4108c532d3f2b84f",
                vec![196, 1, 128, 128, 128],
            ),
            (
                "0x2374954008440ca3d17b1472d34cc52a6493a94fb490d5fb427184d7d5fd1cbf",
                vec![196, 1, 128, 128, 128],
            ),
            (
                "0x23ddaac09188c12e5d88009afa4a34041175c5531f45be53f1560a1cbfec4e8a",
                vec![
                    228, 1, 128, 160, 71, 250, 72, 226, 93, 54, 105, 169, 187, 25, 12, 89, 147,
                    143, 75, 228, 157, 226, 208, 131, 105, 110, 185, 57, 195, 180, 7, 46, 198, 126,
                    67, 177, 128,
                ],
            ),
            (
                "0x246cc8a2b79a30ec71390d829d0cb37cce1b953e89cb14deae4945526714a71c",
                vec![196, 128, 1, 128, 128],
            ),
            (
                "0x255ec86eac03ba59f6dfcaa02128adbb22c561ae0c49e9e62e4fff363750626e",
                vec![
                    228, 1, 128, 160, 102, 235, 22, 7, 27, 163, 121, 191, 12, 99, 47, 203, 82, 249,
                    23, 90, 101, 107, 239, 98, 173, 240, 190, 245, 52, 154, 127, 90, 106, 173, 93,
                    136, 128,
                ],
            ),
            (
                "0x26ce7d83dfb0ab0e7f15c42aeb9e8c0c5dba538b07c8e64b35fb64a37267dd96",
                vec![
                    228, 1, 128, 160, 36, 52, 191, 198, 67, 236, 54, 65, 22, 205, 113, 81, 154, 57,
                    118, 98, 178, 12, 82, 209, 173, 207, 240, 184, 48, 232, 10, 115, 142, 25, 243,
                    14, 128,
                ],
            ),
            (
                "0x2705244734f69af78e16c74784e1dc921cb8b6a98fe76f577cc441c831e973bf",
                vec![196, 1, 128, 128, 128],
            ),
            (
                "0x28f25652ec67d8df6a2e33730e5d0983443e3f759792a0128c06756e8eb6c37f",
                vec![
                    211, 128, 143, 192, 151, 206, 123, 201, 7, 21, 179, 75, 159, 16, 0, 0, 0, 0,
                    128, 128,
                ],
            ),
            (
                "0x2a248c1755e977920284c8054fceeb20530dc07cd8bbe876f3ce02000818cc3a",
                vec![196, 1, 128, 128, 128],
            ),
            (
                "0x2a39afbe88f572c23c90da2d059af3de125f1da5c3753c530dc5619a4857119f",
                vec![
                    228, 1, 128, 160, 130, 137, 181, 88, 134, 95, 44, 161, 245, 76, 152, 181, 255,
                    93, 249, 95, 7, 194, 78, 198, 5, 226, 71, 181, 140, 119, 152, 96, 93, 205, 121,
                    79, 128,
                ],
            ),
            (
                "0x2b8d12301a8af18405b3c826b6edcc60e8e034810f00716ca48bebb84c4ce7ab",
                vec![196, 1, 128, 128, 128],
            ),
            (
                "0x2baa718b760c0cbd0ec40a3c6df7f2948b40ba096e6e4b116b636f0cca023bde",
                vec![196, 128, 1, 128, 128],
            ),
            (
                "0x2e6fe1362b3e388184fd7bf08e99e74170b26361624ffd1c5f646da7067b58b6",
                vec![
                    228, 128, 128, 128, 160, 142, 3, 136, 236, 246, 76, 250, 118, 179, 166, 175,
                    21, 159, 119, 69, 21, 25, 167, 249, 187, 134, 46, 76, 206, 36, 23, 92, 121, 31,
                    220, 176, 223,
                ],
            ),
            (
                "0x2fe5767f605b7b821675b223a22e4e5055154f75e7f3041fdffaa02e4787fab8",
                vec![196, 128, 1, 128, 128],
            ),
            (
                "0x303f57a0355c50bf1a0e1cf0fa8f9bdbc8d443b70f2ad93ac1c6b9c1d1fe29a2",
                vec![201, 128, 133, 23, 72, 118, 232, 0, 128, 128],
            ),
            (
                "0x30ce5b7591126d5464dfb4fc576a970b1368475ce097e244132b06d8cc8ccffe",
                vec![196, 128, 1, 128, 128],
            ),
            (
                "0x315ccc15883d06b4e743f8252c999bf1ee994583ff6114d89c0f3ddee828302b",
                vec![196, 1, 128, 128, 128],
            ),
            (
                "0x3197690074092fe51694bdb96aaab9ae94dac87f129785e498ab171a363d3b40",
                vec![196, 128, 1, 128, 128],
            ),
            (
                "0x34a715e08b77afd68cde30b62e222542f3db90758370400c94d0563959a1d1a0",
                vec![
                    228, 1, 128, 160, 79, 68, 99, 41, 181, 238, 61, 19, 212, 246, 181, 229, 242,
                    16, 221, 194, 217, 15, 237, 186, 56, 75, 149, 14, 54, 161, 209, 154, 249, 92,
                    92, 177, 128,
                ],
            ),
            (
                "0x37310559ceaade42e45b3e3f05925aadca9e60aeeb9dd60d824875d9e9e71e26",
                vec![
                    228, 1, 128, 160, 114, 200, 146, 33, 218, 237, 204, 221, 63, 187, 166, 108, 27,
                    8, 27, 54, 52, 206, 137, 213, 160, 105, 190, 151, 255, 120, 50, 119, 143, 123,
                    2, 58, 128,
                ],
            ),
            (
                "0x37d65eaa92c6bc4c13a5ec45527f0c18ea8932588728769ec7aecfe6d9f32e42",
                vec![
                    248, 68, 128, 42, 160, 172, 49, 98, 168, 185, 219, 180, 49, 139, 132, 33, 159,
                    49, 64, 231, 169, 236, 53, 18, 98, 52, 18, 2, 151, 221, 225, 15, 81, 178, 95,
                    106, 38, 160, 245, 122, 205, 64, 37, 152, 114, 96, 109, 118, 25, 126, 240, 82,
                    243, 211, 85, 136, 218, 223, 145, 158, 225, 240, 227, 203, 155, 98, 211, 244,
                    176, 44,
                ],
            ),
            (
                "0x37ddfcbcb4b2498578f90e0fcfef9965dcde4d4dfabe2f2836d2257faa169947",
                vec![
                    228, 1, 128, 160, 82, 214, 210, 145, 58, 228, 75, 202, 17, 181, 161, 22, 2, 29,
                    185, 124, 145, 161, 62, 56, 94, 212, 139, 160, 102, 40, 231, 66, 1, 35, 29,
                    186, 128,
                ],
            ),
            (
                "0x37e51740ad994839549a56ef8606d71ace79adc5f55c988958d1c450eea5ac2d",
                vec![196, 1, 128, 128, 128],
            ),
            (
                "0x38152bce526b7e1c2bedfc9d297250fcead02818be7806638564377af145103b",
                vec![
                    228, 1, 128, 160, 108, 0, 224, 145, 218, 227, 212, 34, 111, 172, 214, 190, 128,
                    44, 134, 93, 93, 176, 245, 36, 117, 77, 34, 102, 100, 6, 19, 139, 84, 250, 176,
                    230, 128,
                ],
            ),
            (
                "0x3848b7da914222540b71e398081d04e3849d2ee0d328168a3cc173a1cd4e783b",
                vec![201, 128, 133, 23, 72, 118, 232, 0, 128, 128],
            ),
            (
                "0x389093badcaa24c3a8cbb4461f262fba44c4f178a162664087924e85f3d55710",
                vec![196, 1, 128, 128, 128],
            ),
            (
                "0x3897cb9b6f68765022f3c74f84a9f2833132858f661f4bc91ccd7a98f4e5b1ee",
                vec![196, 1, 128, 128, 128],
            ),
            (
                "0x395b92f75f8e06b5378a84ba03379f025d785d8b626b2b6a1c84b718244b9a91",
                vec![
                    228, 1, 128, 160, 84, 70, 184, 24, 244, 198, 105, 102, 156, 211, 49, 71, 38,
                    255, 19, 76, 241, 140, 88, 169, 165, 54, 223, 19, 199, 0, 97, 7, 5, 168, 183,
                    200, 128,
                ],
            ),
            (
                "0x3be526914a7d688e00adca06a0c47c580cb7aa934115ca26006a1ed5455dd2ce",
                vec![196, 128, 1, 128, 128],
            ),
            (
                "0x3e57e37bc3f588c244ffe4da1f48a360fa540b77c92f0c76919ec4ee22b63599",
                vec![196, 128, 1, 128, 128],
            ),
            (
                "0x415ded122ff7b6fe5862f5c443ea0375e372862b9001c5fe527d276a3a420280",
                vec![196, 1, 128, 128, 128],
            ),
            (
                "0x419809ad1512ed1ab3fb570f98ceb2f1d1b5dea39578583cd2b03e9378bbe418",
                vec![196, 1, 128, 128, 128],
            ),
            (
                "0x4363d332a0d4df8582a84932729892387c623fe1ec42e2cfcbe85c183ed98e0e",
                vec![
                    213, 130, 1, 146, 143, 192, 151, 206, 123, 201, 7, 21, 179, 73, 233, 122, 138,
                    101, 46, 31, 128, 128,
                ],
            ),
            (
                "0x445cb5c1278fdce2f9cbdb681bdd76c52f8e50e41dbd9e220242a69ba99ac099",
                vec![
                    228, 1, 1, 160, 190, 61, 117, 161, 114, 155, 225, 87, 231, 156, 59, 119, 240,
                    2, 6, 219, 77, 84, 227, 234, 20, 55, 90, 1, 84, 81, 200, 142, 192, 103, 199,
                    144, 128,
                ],
            ),
            (
                "0x4615e5f5df5b25349a00ad313c6cd0436b6c08ee5826e33a018661997f85ebaa",
                vec![196, 1, 128, 128, 128],
            ),
            (
                "0x465311df0bf146d43750ed7d11b0451b5f6d5bfc69b8a216ef2f1c79c93cd848",
                vec![196, 128, 1, 128, 128],
            ),
            (
                "0x47450e5beefbd5e3a3f80cbbac474bb3db98d5e609aa8d15485c3f0d733dea3a",
                vec![
                    228, 1, 128, 160, 84, 66, 224, 39, 157, 63, 17, 73, 222, 76, 232, 217, 226,
                    211, 240, 29, 24, 84, 117, 80, 56, 172, 26, 15, 174, 92, 72, 116, 155, 247, 31,
                    32, 128,
                ],
            ),
            (
                "0x482814ea8f103c39dcf6ba7e75df37145bde813964d82e81e5d7e3747b95303d",
                vec![196, 128, 1, 128, 128],
            ),
            (
                "0x4845aac9f26fcd628b39b83d1ccb5c554450b9666b66f83aa93a1523f4db0ab6",
                vec![201, 128, 133, 23, 72, 118, 232, 0, 128, 128],
            ),
            (
                "0x48e291f8a256ab15da8401c8cae555d5417a992dff3848926fa5b71655740059",
                vec![
                    228, 1, 128, 160, 162, 231, 8, 75, 169, 206, 193, 121, 81, 156, 126, 137, 80,
                    198, 106, 211, 203, 168, 88, 106, 96, 207, 249, 244, 214, 12, 24, 141, 214, 33,
                    82, 42, 128,
                ],
            ),
            (
                "0x4973f6aa8cf5b1190fc95379aa01cff99570ee6b670725880217237fb49e4b24",
                vec![
                    228, 1, 128, 160, 174, 46, 127, 28, 147, 60, 108, 168, 76, 232, 190, 129, 30,
                    244, 17, 222, 231, 115, 251, 105, 80, 128, 86, 215, 36, 72, 4, 142, 161, 219,
                    92, 71, 128,
                ],
            ),
            (
                "0x4b238e08b80378d0815e109f350a08e5d41ec4094df2cfce7bc8b9e3115bda70",
                vec![
                    228, 1, 128, 160, 17, 245, 211, 153, 202, 143, 183, 169, 175, 90, 212, 129,
                    190, 96, 207, 97, 212, 84, 147, 205, 32, 32, 108, 157, 10, 35, 124, 231, 215,
                    87, 30, 95, 128,
                ],
            ),
            (
                "0x4b9f335ce0bdffdd77fdb9830961c5bc7090ae94703d0392d3f0ff10e6a4fbab",
                vec![201, 128, 133, 23, 72, 118, 232, 0, 128, 128],
            ),
            (
                "0x4bd8ef9873a5e85d4805dbcb0dbf6810e558ea175167549ef80545a9cafbb0e1",
                vec![
                    228, 1, 128, 160, 161, 73, 19, 213, 72, 172, 29, 63, 153, 98, 162, 26, 86, 159,
                    229, 47, 20, 54, 182, 210, 245, 234, 78, 54, 222, 19, 234, 133, 94, 222, 84,
                    224, 128,
                ],
            ),
            (
                "0x4c2765139cace1d217e238cc7ccfbb751ef200e0eae7ec244e77f37e92dfaee5",
                vec![196, 1, 128, 128, 128],
            ),
            (
                "0x4c310e1f5d2f2e03562c4a5c473ae044b9ee19411f07097ced41e85bd99c3364",
                vec![196, 128, 1, 128, 128],
            ),
            (
                "0x4ccd31891378d2025ef58980481608f11f5b35a988e877652e7cbb0a6127287c",
                vec![201, 128, 133, 23, 72, 118, 232, 0, 128, 128],
            ),
            (
                "0x4ceaf2371fcfb54a4d8bc1c804d90b06b3c32c9f17112b57c29b30a25cf8ca12",
                vec![196, 128, 1, 128, 128],
            ),
        ];

        // Create a store and load it up with the accounts
        let store = Store::new("null", EngineType::InMemory).unwrap();
        let mut state_trie = store.new_state_trie_for_test()?;
        for (address, account) in accounts {
            let hashed_address = H256::from_str(address).unwrap().as_bytes().to_vec();
            let account = AccountState::from(AccountStateSlim::decode(&account).unwrap());
            state_trie
                .insert(hashed_address, account.encode_to_vec())
                .unwrap();
        }
        Ok((store, state_trie.hash().unwrap()))
    }
}
