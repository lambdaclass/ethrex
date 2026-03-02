// Copyright 2015 Jeffrey Wilcke, Felix Lange, Gustav Simonsson. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be found in
// the LICENSE file.

// secp256k1_context_create_sign_verify creates a context for signing and signature verification.
static secp256k1_context* secp256k1_context_create_sign_verify() {
	return secp256k1_context_create(SECP256K1_CONTEXT_SIGN | SECP256K1_CONTEXT_VERIFY);
}

// secp256k1_ext_ecdsa_recover recovers the public key of an encoded compact signature.
//
// Returns: 1: recovery was successful
//          0: recovery was not successful
// Args:    ctx:        pointer to a context object (cannot be NULL)
//  Out:    pubkey_out: the serialized 65-byte public key of the signer (cannot be NULL)
//  In:     sigdata:    pointer to a 65-byte signature with the recovery id at the end (cannot be NULL)
//          msgdata:    pointer to a 32-byte message (cannot be NULL)
static int secp256k1_ext_ecdsa_recover(
	const secp256k1_context* ctx,
	unsigned char *pubkey_out,
	const unsigned char *sigdata,
	const unsigned char *msgdata
) {
	secp256k1_ecdsa_recoverable_signature sig;
	secp256k1_pubkey pubkey;

	if (!secp256k1_ecdsa_recoverable_signature_parse_compact(ctx, &sig, sigdata, (int)sigdata[64])) {
		return 0;
	}
	if (!secp256k1_ecdsa_recover(ctx, &pubkey, &sig, msgdata)) {
		return 0;
	}
	size_t outputlen = 65;
	return secp256k1_ec_pubkey_serialize(ctx, pubkey_out, &outputlen, &pubkey, SECP256K1_EC_UNCOMPRESSED);
}
