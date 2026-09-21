/// Integration tests for EIP-8070 blob reconstruction (Phase B).
///
/// Tests cover:
///   T3  – reconstruct_blobs_bundle succeeds when all 128 columns are stored.
///   T3b – reconstruct_blobs_bundle returns Ok(None) when only 63 columns are stored.
///   T4  – the reconstructed bundle passes BlobsBundle::validate.
#[cfg(feature = "c-kzg")]
mod eip8070_kzg {
    use ethrex_blockchain::mempool::Mempool;
    use ethrex_common::{
        H256,
        types::{
            BYTES_PER_BLOB, BYTES_PER_CELL, BlobsBundle, CELLS_PER_EXT_BLOB, EIP4844Transaction,
            Fork, kzg_commitment_to_versioned_hash,
        },
    };
    use ethrex_crypto::kzg::compute_cells;

    fn h(n: u64) -> H256 {
        H256::from_low_u64_be(n)
    }

    /// Build a valid sample blob where every 32-byte field element stays below the
    /// BLS12-381 field modulus (bytes 0-27 are zero; bytes 28-31 hold a small int).
    fn sample_blob() -> [u8; BYTES_PER_BLOB] {
        let mut blob = [0u8; BYTES_PER_BLOB];
        for i in 0..4096usize {
            blob[i * 32 + 28] = (i & 0xFF) as u8;
            blob[i * 32 + 31] = ((i >> 8) & 0xFF) as u8;
        }
        blob
    }

    /// T3: storing all 128 columns for an elided bundle allows reconstruct_blobs_bundle
    /// to return Some with the correct blob content.
    #[test]
    fn reconstruct_full_bundle_from_stored_cells() {
        let mp = Mempool::new(64);
        let tx_hash = h(200);

        let blob = sample_blob();
        // Build a full version-1 bundle using the production helper.
        let full_bundle =
            BlobsBundle::create_from_blobs(&vec![blob], Some(1)).expect("create_from_blobs");
        assert_eq!(full_bundle.blobs.len(), 1);
        assert_eq!(full_bundle.commitments.len(), 1);
        assert_eq!(full_bundle.proofs.len(), CELLS_PER_EXT_BLOB);
        assert_eq!(full_bundle.version, 1);

        // Store the ELIDED version: blobs empty, commitments + proofs intact.
        let elided = BlobsBundle {
            blobs: vec![],
            commitments: full_bundle.commitments.clone(),
            proofs: full_bundle.proofs.clone(),
            version: 1,
        };
        mp.add_blobs_bundle(tx_hash, elided)
            .expect("add_blobs_bundle");

        // Compute all 128 cells and store them.
        let cells = compute_cells(&blob).expect("compute_cells");
        assert_eq!(cells.len(), CELLS_PER_EXT_BLOB);
        let cell_entries: Vec<(usize, usize, Box<[u8; BYTES_PER_CELL]>)> = cells
            .iter()
            .enumerate()
            .map(|(col, c)| (0usize, col, Box::new(*c)))
            .collect();
        mp.store_cells(tx_hash, 1, cell_entries)
            .expect("store_cells");

        // Reconstruct and assert.
        let result = mp
            .reconstruct_blobs_bundle(tx_hash)
            .expect("reconstruct_blobs_bundle");
        assert!(result.is_some(), "expected Some but got None");
        let bundle = result.unwrap();

        assert_eq!(bundle.blobs.len(), 1, "should have 1 reconstructed blob");
        assert_eq!(
            bundle.blobs[0], blob,
            "reconstructed blob must match original"
        );
        assert_eq!(
            bundle.commitments, full_bundle.commitments,
            "commitments must be preserved"
        );
        assert_eq!(bundle.version, 1, "version must be preserved");
        assert_eq!(
            bundle.proofs.len(),
            CELLS_PER_EXT_BLOB,
            "cell proofs must be preserved"
        );
    }

    /// T3b: storing only 63 columns (below the recovery threshold) causes
    /// reconstruct_blobs_bundle to return Ok(None).
    #[test]
    fn reconstruct_returns_none_with_63_columns() {
        let mp = Mempool::new(64);
        let tx_hash = h(201);

        let blob = sample_blob();
        let full_bundle =
            BlobsBundle::create_from_blobs(&vec![blob], Some(1)).expect("create_from_blobs");

        let elided = BlobsBundle {
            blobs: vec![],
            commitments: full_bundle.commitments.clone(),
            proofs: full_bundle.proofs.clone(),
            version: 1,
        };
        mp.add_blobs_bundle(tx_hash, elided)
            .expect("add_blobs_bundle");

        // Store only 63 columns (even indices 0, 2, ..., 124 — that's 63 entries).
        let cells = compute_cells(&blob).expect("compute_cells");
        let partial: Vec<(usize, usize, Box<[u8; BYTES_PER_CELL]>)> = (0..63usize)
            .map(|i| {
                let col = i * 2; // even columns: 0, 2, 4, ..., 124
                (0usize, col, Box::new(cells[col]))
            })
            .collect();
        mp.store_cells(tx_hash, 1, partial).expect("store_cells");

        let result = mp
            .reconstruct_blobs_bundle(tx_hash)
            .expect("no error expected");
        assert!(
            result.is_none(),
            "expected Ok(None) with only 63 columns stored"
        );
    }

    /// T4: the bundle returned by reconstruct_blobs_bundle passes BlobsBundle::validate.
    #[test]
    fn reconstructed_bundle_passes_validate() {
        let mp = Mempool::new(64);
        let tx_hash = h(202);

        let blob = sample_blob();
        let full_bundle =
            BlobsBundle::create_from_blobs(&vec![blob], Some(1)).expect("create_from_blobs");

        let versioned_hash = kzg_commitment_to_versioned_hash(&full_bundle.commitments[0]);

        let elided = BlobsBundle {
            blobs: vec![],
            commitments: full_bundle.commitments.clone(),
            proofs: full_bundle.proofs.clone(),
            version: 1,
        };
        mp.add_blobs_bundle(tx_hash, elided)
            .expect("add_blobs_bundle");

        let cells = compute_cells(&blob).expect("compute_cells");
        let cell_entries: Vec<(usize, usize, Box<[u8; BYTES_PER_CELL]>)> = cells
            .iter()
            .enumerate()
            .map(|(col, c)| (0usize, col, Box::new(*c)))
            .collect();
        mp.store_cells(tx_hash, 1, cell_entries)
            .expect("store_cells");

        let bundle = mp
            .reconstruct_blobs_bundle(tx_hash)
            .expect("reconstruct_blobs_bundle")
            .expect("expected Some");

        // Build the matching EIP-4844 tx with the correct versioned hash.
        let eip4844_tx = EIP4844Transaction {
            nonce: 0,
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: 0,
            max_fee_per_blob_gas: 0.into(),
            gas: 21_000,
            to: ethrex_common::Address::from_low_u64_be(1),
            value: ethrex_common::U256::zero(),
            data: ethrex_common::Bytes::default(),
            access_list: Default::default(),
            blob_versioned_hashes: vec![versioned_hash],
            ..Default::default()
        };

        bundle
            .validate(&eip4844_tx, Fork::Osaka)
            .expect("validate must succeed for a correctly reconstructed bundle");
    }
}
