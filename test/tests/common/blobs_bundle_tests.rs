use ethrex_common::types::{
    BYTES_PER_BLOB, BlobsBundle, BlobsBundleError, CELLS_PER_EXT_BLOB, Fork,
    blobs_bundle::blob_from_bytes,
};

/// Helper: create a valid v1 blob bundle with one blob.
fn valid_bundle_and_tx() -> (BlobsBundle, ethrex_common::types::EIP4844Transaction) {
    let blobs = vec![blob_from_bytes("Hello, world!".as_bytes().into()).unwrap()];
    let bundle = BlobsBundle::create_from_blobs(&blobs).unwrap();
    let blob_versioned_hashes = bundle.generate_versioned_hashes();

    let tx = ethrex_common::types::EIP4844Transaction {
        blob_versioned_hashes,
        ..Default::default()
    };
    (bundle, tx)
}

#[test]
fn validate_cheap_accepts_valid_blobs() {
    let (bundle, tx) = valid_bundle_and_tx();
    assert!(bundle.validate_cheap(&tx, Fork::Prague).is_ok());
}

#[test]
fn validate_cheap_rejects_empty_bundle() {
    let bundle = BlobsBundle::empty();
    let tx = ethrex_common::types::EIP4844Transaction::default();

    assert!(matches!(
        bundle.validate_cheap(&tx, Fork::Prague),
        Err(BlobsBundleError::BlobBundleEmptyError)
    ));
}

#[test]
fn validate_cheap_rejects_wrong_lengths() {
    // Bundle with 1 blob but 2 commitments
    let blobs = vec![blob_from_bytes("data".as_bytes().into()).unwrap()];
    let mut bundle = BlobsBundle::create_from_blobs(&blobs).unwrap();
    let blob_versioned_hashes = bundle.generate_versioned_hashes();

    let tx = ethrex_common::types::EIP4844Transaction {
        blob_versioned_hashes,
        ..Default::default()
    };

    // Add an extra commitment to cause a length mismatch
    bundle.commitments.push([0u8; 48]);

    assert!(matches!(
        bundle.validate_cheap(&tx, Fork::Prague),
        Err(BlobsBundleError::BlobsBundleWrongLen)
    ));
}

#[test]
fn validate_cheap_rejects_version_fork_mismatch() {
    // v0 bundles are always rejected — only version 1 is valid regardless of fork.
    let blobs = vec![blob_from_bytes("Hello, world!".as_bytes().into()).unwrap()];
    let v1_bundle = BlobsBundle::create_from_blobs(&blobs).unwrap();
    // Manually construct a v0 bundle (single proof per blob).
    let bundle = BlobsBundle {
        blobs: v1_bundle.blobs.clone(),
        commitments: v1_bundle.commitments.clone(),
        proofs: vec![[0u8; 48]],
        version: 0,
    };
    let blob_versioned_hashes = bundle.generate_versioned_hashes();
    let tx = ethrex_common::types::EIP4844Transaction {
        blob_versioned_hashes,
        ..Default::default()
    };
    // Rejected on Prague
    assert!(matches!(
        bundle.validate_cheap(&tx, Fork::Prague),
        Err(BlobsBundleError::InvalidBlobVersionForFork)
    ));
    // Also rejected on Osaka
    assert!(matches!(
        bundle.validate_cheap(&tx, Fork::Osaka),
        Err(BlobsBundleError::InvalidBlobVersionForFork)
    ));
}

#[test]
fn validate_cheap_rejects_wrong_versioned_hashes() {
    let (bundle, mut tx) = valid_bundle_and_tx();
    // Replace the versioned hash with a bogus one
    tx.blob_versioned_hashes = vec![ethrex_common::H256::zero()];

    assert!(matches!(
        bundle.validate_cheap(&tx, Fork::Prague),
        Err(BlobsBundleError::BlobVersionedHashesError)
    ));
}

#[test]
fn validate_cheap_passes_with_invalid_kzg_proofs() {
    // Create a v1 bundle with valid structure but zeroed-out cell proofs.
    // validate_cheap() should pass (no KZG check), validate() should fail.
    let blobs = vec![[0u8; BYTES_PER_BLOB]];
    let valid_bundle = BlobsBundle::create_from_blobs(&blobs).unwrap();

    // Use valid commitments (versioned hashes remain correct) but zero all 128 cell proofs.
    let bundle = BlobsBundle {
        blobs: blobs.clone(),
        commitments: valid_bundle.commitments.clone(),
        proofs: vec![[0u8; 48]; CELLS_PER_EXT_BLOB],
        version: 1,
    };

    let blob_versioned_hashes = bundle.generate_versioned_hashes();
    let tx = ethrex_common::types::EIP4844Transaction {
        blob_versioned_hashes,
        ..Default::default()
    };

    // Cheap validation passes (correct version, correct lengths, correct hashes).
    assert!(
        bundle.validate_cheap(&tx, Fork::Prague).is_ok(),
        "validate_cheap should pass with structurally valid but KZG-invalid bundle"
    );

    // Full validation fails on KZG cell proof verification.
    let result = bundle.validate(&tx, Fork::Prague);
    assert!(
        result.is_err(),
        "validate should fail for invalid KZG proofs, but got Ok"
    );
    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            BlobsBundleError::BlobToCommitmentAndProofError | BlobsBundleError::Kzg(_)
        ),
        "validate should fail with a KZG-related error, got: {err:?}"
    );
}

/// Regression (`kzg-sidecar-constraints`): EIP-7594 limits a single transaction to 6
/// blobs. A 7-blob transaction must be rejected on Osaka, even though the per-block
/// limit is higher.
#[test]
fn validate_cheap_rejects_more_than_six_blobs_per_tx_on_osaka() {
    // Build one valid Osaka (version 1) blob, then replicate it to 7 blobs so the
    // bundle stays structurally valid (matching commitments / cell-proofs / hashes)
    // and the only thing wrong is the per-transaction blob count.
    let one = BlobsBundle::create_from_blobs(&vec![[0u8; BYTES_PER_BLOB]]).unwrap();
    let bundle = BlobsBundle {
        blobs: vec![one.blobs[0]; 7],
        commitments: vec![one.commitments[0]; 7],
        proofs: one.proofs.repeat(7),
        version: 1,
    };
    let tx = ethrex_common::types::EIP4844Transaction {
        blob_versioned_hashes: bundle.generate_versioned_hashes(),
        ..Default::default()
    };

    assert!(
        bundle.validate_cheap(&tx, Fork::Osaka).is_err(),
        "a transaction with more than 6 blobs must be rejected on Osaka (EIP-7594)"
    );
}

/// Regression (`kzg-sidecar-constraints`): on Osaka the only valid blob wrapper
/// version is 1. A version-2 bundle must be rejected, not merely any non-zero version.
#[test]
fn validate_cheap_rejects_noncanonical_wrapper_version_on_osaka() {
    // Build a structurally valid v1 bundle then bump version to 2.
    let blobs = vec![[0u8; BYTES_PER_BLOB]];
    let v1_bundle = BlobsBundle::create_from_blobs(&blobs).unwrap();
    let bundle = BlobsBundle {
        version: 2,
        ..v1_bundle
    };
    let tx = ethrex_common::types::EIP4844Transaction {
        blob_versioned_hashes: bundle.generate_versioned_hashes(),
        ..Default::default()
    };

    assert!(
        matches!(
            bundle.validate_cheap(&tx, Fork::Osaka),
            Err(BlobsBundleError::InvalidBlobVersionForFork)
        ),
        "wrapper version 2 must be rejected on Osaka (only version 1 is valid)"
    );
}
