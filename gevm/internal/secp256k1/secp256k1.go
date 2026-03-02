// Copyright 2015 Jeffrey Wilcke, Felix Lange, Gustav Simonsson. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be found in
// the LICENSE file.

//go:build cgo

// Package secp256k1 wraps the bitcoin secp256k1 C library.
// Only RecoverPubkey is exposed — this is the sole function needed by the
// ECRECOVER precompile.
package secp256k1

/*
#cgo CFLAGS: -I./libsecp256k1
#cgo CFLAGS: -I./libsecp256k1/src/

#ifndef NDEBUG
#  define NDEBUG
#endif

#include "./libsecp256k1/src/secp256k1.c"
#include "./libsecp256k1/src/modules/recovery/main_impl.h"
#include "./libsecp256k1/src/precomputed_ecmult.c"
#include "./libsecp256k1/src/precomputed_ecmult_gen.c"
#include "ext.h"

typedef void (*callbackFunc) (const char* msg, void* data);
extern void secp256k1GoPanicIllegal(const char* msg, void* data);
extern void secp256k1GoPanicError(const char* msg, void* data);
*/
import "C"

import (
	"errors"
	"unsafe"
)

var context *C.secp256k1_context

func init() {
	context = C.secp256k1_context_create_sign_verify()
	C.secp256k1_context_set_illegal_callback(context, C.callbackFunc(C.secp256k1GoPanicIllegal), nil)
	C.secp256k1_context_set_error_callback(context, C.callbackFunc(C.secp256k1GoPanicError), nil)
}

var (
	ErrInvalidMsgLen       = errors.New("invalid message length, need 32 bytes")
	ErrInvalidSignatureLen = errors.New("invalid signature length")
	ErrInvalidRecoveryID   = errors.New("invalid signature recovery id")
	ErrRecoverFailed       = errors.New("recovery failed")
)

// RecoverPubkey returns the uncompressed public key of the signer.
// msg must be the 32-byte hash of the message to be signed.
// sig must be a 65-byte compact ECDSA signature containing the
// recovery id as the last element.
func RecoverPubkey(msg []byte, sig []byte) ([]byte, error) {
	if len(msg) != 32 {
		return nil, ErrInvalidMsgLen
	}
	if len(sig) != 65 {
		return nil, ErrInvalidSignatureLen
	}
	if sig[64] >= 4 {
		return nil, ErrInvalidRecoveryID
	}

	var (
		pubkey  = make([]byte, 65)
		sigdata = (*C.uchar)(unsafe.Pointer(&sig[0]))
		msgdata = (*C.uchar)(unsafe.Pointer(&msg[0]))
	)
	if C.secp256k1_ext_ecdsa_recover(context, (*C.uchar)(unsafe.Pointer(&pubkey[0])), sigdata, msgdata) == 0 {
		return nil, ErrRecoverFailed
	}
	return pubkey, nil
}
