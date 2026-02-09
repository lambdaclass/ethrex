use ethrex_common::types::{
    blobs_bundle::blob_from_bytes, BlobsBundle, BlobsBundleError, Fork, BYTES_PER_BLOB,
};

/// Helper: create a valid blob bundle with one blob for pre-Osaka (version 0).
fn valid_bundle_and_tx() -> (BlobsBundle, ethrex_common::types::EIP4844Transaction) {
    let blobs = vec![blob_from_bytes("Hello, world!".as_bytes().into()).unwrap()];
    let bundle = BlobsBundle::create_from_blobs(&blobs, None).unwrap();
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
    let mut bundle = BlobsBundle::create_from_blobs(&blobs, None).unwrap();
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
    // version-0 bundle on Osaka fork should fail
    let (bundle, tx) = valid_bundle_and_tx();
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
    // Create a bundle with valid structure but invalid KZG proofs.
    // validate_cheap() should pass (KZG skipped), validate() should fail.
    let blobs = vec![[0u8; BYTES_PER_BLOB]];
    let valid_bundle = BlobsBundle::create_from_blobs(&blobs, None).unwrap();

    // Use valid commitments but zero out proofs to make KZG invalid
    let bundle = BlobsBundle {
        blobs: blobs.clone(),
        commitments: valid_bundle.commitments.clone(),
        proofs: vec![[0u8; 48]], // invalid proof
        version: 0,
    };

    let blob_versioned_hashes = bundle.generate_versioned_hashes();
    let tx = ethrex_common::types::EIP4844Transaction {
        blob_versioned_hashes,
        ..Default::default()
    };

    // Cheap validation passes (no KZG check)
    assert!(
        bundle.validate_cheap(&tx, Fork::Prague).is_ok(),
        "validate_cheap should pass with structurally valid but KZG-invalid bundle"
    );

    // Full validation fails on KZG proof
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
