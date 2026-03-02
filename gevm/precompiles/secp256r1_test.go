package precompiles

import (
	"encoding/hex"
	"testing"
)

func TestP256Verify(t *testing.T) {
	// Test vectors from https://github.com/daimo-eth/p256-verifier/tree/master/test-vectors
	tests := []struct {
		name    string
		input   string
		success bool
	}{
		// Valid signatures
		{
			name:    "ok_1",
			input:   "4cee90eb86eaa050036147a12d49004b6b9c72bd725d39d4785011fe190f0b4da73bd4903f0ce3b639bbbf6e8e80d16931ff4bcf5993d58468e8fb19086e8cac36dbcd03009df8c59286b162af3bd7fcc0450c9aa81be5d10d312af6c66b1d604aebd3099c618202fcfe16ae7770b0c49ab5eadf74b754204a3bb6060e44eff37618b065f9832de4ca6ca971a7a1adc826d0f7c00181a5fb2ddf79ae00b4e10e",
			success: true,
		},
		{
			name:    "ok_2",
			input:   "3fec5769b5cf4e310a7d150508e82fb8e3eda1c2c94c61492d3bd8aea99e06c9e22466e928fdccef0de49e3503d2657d00494a00e764fd437bdafa05f5922b1fbbb77c6817ccf50748419477e843d5bac67e6a70e97dde5a57e0c983b777e1ad31a80482dadf89de6302b1988c82c29544c9c07bb910596158f6062517eb089a2f54c9a0f348752950094d3228d3b940258c75fe2a413cb70baa21dc2e352fc5",
			success: true,
		},
		{
			name:    "ok_3",
			input:   "e775723953ead4a90411a02908fd1a629db584bc600664c609061f221ef6bf7c440066c8626b49daaa7bf2bcc0b74be4f7a1e3dcf0e869f1542fe821498cbf2de73ad398194129f635de4424a07ca715838aefe8fe69d1a391cfa70470795a80dd056866e6e1125aff94413921880c437c9e2570a28ced7267c8beef7e9b2d8d1547d76dfcf4bee592f5fefe10ddfb6aeb0991c5b9dbbee6ec80d11b17c0eb1a",
			success: true,
		},
		{
			name:    "ok_4",
			input:   "b5a77e7a90aa14e0bf5f337f06f597148676424fae26e175c6e5621c34351955289f319789da424845c9eac935245fcddd805950e2f02506d09be7e411199556d262144475b1fa46ad85250728c600c53dfd10f8b3f4adf140e27241aec3c2da3a81046703fccf468b48b145f939efdbb96c3786db712b3113bb2488ef286cdcef8afe82d200a5bb36b5462166e8ce77f2d831a52ef2135b2af188110beaefb1",
			success: true,
		},
		{
			name:    "ok_5",
			input:   "858b991cfd78f16537fe6d1f4afd10273384db08bdfc843562a22b0626766686f6aec8247599f40bfe01bec0e0ecf17b4319559022d4d9bf007fe929943004eb4866760dedf31b7c691f5ce665f8aae0bda895c23595c834fecc2390a5bcc203b04afcacbb4280713287a2d0c37e23f7513fab898f2c1fefa00ec09a924c335d9b629f1d4fb71901c3e59611afbfea354d101324e894c788d1c01f00b3c251b2",
			success: true,
		},
		// Wrong message (first byte flipped)
		{
			name:    "fail_wrong_msg_1",
			input:   "3cee90eb86eaa050036147a12d49004b6b9c72bd725d39d4785011fe190f0b4da73bd4903f0ce3b639bbbf6e8e80d16931ff4bcf5993d58468e8fb19086e8cac36dbcd03009df8c59286b162af3bd7fcc0450c9aa81be5d10d312af6c66b1d604aebd3099c618202fcfe16ae7770b0c49ab5eadf74b754204a3bb6060e44eff37618b065f9832de4ca6ca971a7a1adc826d0f7c00181a5fb2ddf79ae00b4e10e",
			success: false,
		},
		{
			name:    "fail_wrong_msg_2",
			input:   "afec5769b5cf4e310a7d150508e82fb8e3eda1c2c94c61492d3bd8aea99e06c9e22466e928fdccef0de49e3503d2657d00494a00e764fd437bdafa05f5922b1fbbb77c6817ccf50748419477e843d5bac67e6a70e97dde5a57e0c983b777e1ad31a80482dadf89de6302b1988c82c29544c9c07bb910596158f6062517eb089a2f54c9a0f348752950094d3228d3b940258c75fe2a413cb70baa21dc2e352fc5",
			success: false,
		},
		{
			name:    "fail_wrong_msg_3",
			input:   "f775723953ead4a90411a02908fd1a629db584bc600664c609061f221ef6bf7c440066c8626b49daaa7bf2bcc0b74be4f7a1e3dcf0e869f1542fe821498cbf2de73ad398194129f635de4424a07ca715838aefe8fe69d1a391cfa70470795a80dd056866e6e1125aff94413921880c437c9e2570a28ced7267c8beef7e9b2d8d1547d76dfcf4bee592f5fefe10ddfb6aeb0991c5b9dbbee6ec80d11b17c0eb1a",
			success: false,
		},
		{
			name:    "fail_wrong_msg_4",
			input:   "c5a77e7a90aa14e0bf5f337f06f597148676424fae26e175c6e5621c34351955289f319789da424845c9eac935245fcddd805950e2f02506d09be7e411199556d262144475b1fa46ad85250728c600c53dfd10f8b3f4adf140e27241aec3c2da3a81046703fccf468b48b145f939efdbb96c3786db712b3113bb2488ef286cdcef8afe82d200a5bb36b5462166e8ce77f2d831a52ef2135b2af188110beaefb1",
			success: false,
		},
		{
			name:    "fail_wrong_msg_5",
			input:   "958b991cfd78f16537fe6d1f4afd10273384db08bdfc843562a22b0626766686f6aec8247599f40bfe01bec0e0ecf17b4319559022d4d9bf007fe929943004eb4866760dedf31b7c691f5ce665f8aae0bda895c23595c834fecc2390a5bcc203b04afcacbb4280713287a2d0c37e23f7513fab898f2c1fefa00ec09a924c335d9b629f1d4fb71901c3e59611afbfea354d101324e894c788d1c01f00b3c251b2",
			success: false,
		},
		// Short input
		{
			name:    "fail_short_input_1",
			input:   "4cee90eb86eaa050036147a12d49004b6a",
			success: false,
		},
		{
			name:    "fail_short_input_2",
			input:   "4cee90eb86eaa050036147a12d49004b6a958b991cfd78f16537fe6d1f4afd10273384db08bdfc843562a22b0626766686f6aec8247599f40bfe01bec0e0ecf17b4319559022d4d9bf007fe929943004eb4866760dedf319",
			success: false,
		},
		// Long input (161 bytes)
		{
			name:    "fail_long_input",
			input:   "4cee90eb86eaa050036147a12d49004b6b9c72bd725d39d4785011fe190f0b4da73bd4903f0ce3b639bbbf6e8e80d16931ff4bcf5993d58468e8fb19086e8cac36dbcd03009df8c59286b162af3bd7fcc0450c9aa81be5d10d312af6c66b1d604aebd3099c618202fcfe16ae7770b0c49ab5eadf74b754204a3bb6060e44eff37618b065f9832de4ca6ca971a7a1adc826d0f7c00181a5fb2ddf79ae00b4e10e00",
			success: false,
		},
		// Invalid signature (r and s set to all 0xff)
		{
			name:    "fail_invalid_sig",
			input:   "4cee90eb86eaa050036147a12d49004b6b9c72bd725d39d4785011fe190f0b4dffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff4aebd3099c618202fcfe16ae7770b0c49ab5eadf74b754204a3bb6060e44eff37618b065f9832de4ca6ca971a7a1adc826d0f7c00181a5fb2ddf79ae00b4e10e",
			success: false,
		},
		// Invalid public key (zeroed out)
		{
			name:    "fail_invalid_pubkey",
			input:   "4cee90eb86eaa050036147a12d49004b6b9c72bd725d39d4785011fe190f0b4da73bd4903f0ce3b639bbbf6e8e80d16931ff4bcf5993d58468e8fb19086e8cac36dbcd03009df8c59286b162af3bd7fcc0450c9aa81be5d10d312af6c66b1d6000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
			success: false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			input, err := hex.DecodeString(tt.input)
			if err != nil {
				t.Fatalf("decode input: %v", err)
			}

			result := P256VerifyRun(input, 3500)
			if result.IsErr() {
				t.Fatalf("unexpected error: %v", *result.Err)
			}
			if result.Output.GasUsed != 3450 {
				t.Fatalf("gas used: got %d, want 3450", result.Output.GasUsed)
			}

			if tt.success {
				if len(result.Output.Bytes) != 32 {
					t.Fatalf("expected 32-byte output, got %d bytes", len(result.Output.Bytes))
				}
				if result.Output.Bytes[31] != 1 {
					t.Fatalf("expected last byte = 1, got %d", result.Output.Bytes[31])
				}
				// Check all other bytes are zero
				for i := 0; i < 31; i++ {
					if result.Output.Bytes[i] != 0 {
						t.Fatalf("expected byte %d = 0, got %d", i, result.Output.Bytes[i])
					}
				}
			} else {
				if len(result.Output.Bytes) != 0 {
					t.Fatalf("expected empty output, got %d bytes", len(result.Output.Bytes))
				}
			}
		})
	}
}

func TestP256VerifyOutOfGas(t *testing.T) {
	input, _ := hex.DecodeString("4cee90eb86eaa050036147a12d49004b6b9c72bd725d39d4785011fe190f0b4da73bd4903f0ce3b639bbbf6e8e80d16931ff4bcf5993d58468e8fb19086e8cac36dbcd03009df8c59286b162af3bd7fcc0450c9aa81be5d10d312af6c66b1d604aebd3099c618202fcfe16ae7770b0c49ab5eadf74b754204a3bb6060e44eff37618b065f9832de4ca6ca971a7a1adc826d0f7c00181a5fb2ddf79ae00b4e10e")

	result := P256VerifyRun(input, 2500)
	if !result.IsErr() {
		t.Fatal("expected out of gas error")
	}
	if *result.Err != PrecompileErrorOutOfGas {
		t.Fatalf("expected OutOfGas, got %d", *result.Err)
	}
}

func TestP256VerifyOsaka(t *testing.T) {
	// Valid signature, Osaka gas cost
	input, _ := hex.DecodeString("b5a77e7a90aa14e0bf5f337f06f597148676424fae26e175c6e5621c34351955289f319789da424845c9eac935245fcddd805950e2f02506d09be7e411199556d262144475b1fa46ad85250728c600c53dfd10f8b3f4adf140e27241aec3c2da3a81046703fccf468b48b145f939efdbb96c3786db712b3113bb2488ef286cdcef8afe82d200a5bb36b5462166e8ce77f2d831a52ef2135b2af188110beaefb1")

	// Should fail with insufficient gas for Osaka (6900)
	result := P256VerifyOsakaRun(input, 5000)
	if !result.IsErr() {
		t.Fatal("expected out of gas error for Osaka with 5000 gas")
	}

	// Should succeed with sufficient gas for Osaka
	result = P256VerifyOsakaRun(input, 7000)
	if result.IsErr() {
		t.Fatalf("unexpected error: %v", *result.Err)
	}
	if result.Output.GasUsed != 6900 {
		t.Fatalf("gas used: got %d, want 6900", result.Output.GasUsed)
	}
	if len(result.Output.Bytes) != 32 || result.Output.Bytes[31] != 1 {
		t.Fatal("expected success output (B256 with last byte 1)")
	}
}

func TestP256VerifyImpl(t *testing.T) {
	// Additional verify_impl tests
	tests := []struct {
		name    string
		input   string
		success bool
	}{
		{
			name:    "ok_1",
			input:   "b5a77e7a90aa14e0bf5f337f06f597148676424fae26e175c6e5621c34351955289f319789da424845c9eac935245fcddd805950e2f02506d09be7e411199556d262144475b1fa46ad85250728c600c53dfd10f8b3f4adf140e27241aec3c2da3a81046703fccf468b48b145f939efdbb96c3786db712b3113bb2488ef286cdcef8afe82d200a5bb36b5462166e8ce77f2d831a52ef2135b2af188110beaefb1",
			success: true,
		},
		{
			name:    "fail_wrong_pubkey_x",
			input:   "b5a77e7a90aa14e0bf5f337f06f597148676424fae26e175c6e5621c34351955289f319789da424845c9eac935245fcddd805950e2f02506d09be7e411199556d262144475b1fa46ad85250728c600c53dfd10f8b3f4adf140e27241aec3c2daaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaef8afe82d200a5bb36b5462166e8ce77f2d831a52ef2135b2af188110beaefb1",
			success: false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			input, err := hex.DecodeString(tt.input)
			if err != nil {
				t.Fatalf("decode: %v", err)
			}
			got := p256VerifyImpl(input)
			if got != tt.success {
				t.Fatalf("p256VerifyImpl: got %v, want %v", got, tt.success)
			}
		})
	}
}
