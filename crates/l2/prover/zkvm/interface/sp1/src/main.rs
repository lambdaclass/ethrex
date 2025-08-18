#![no_main]

use ethrex_common::Bytes;
use ethrex_common::U256;
use ethrex_common::utils::u256_from_big_endian;
use keccak_hash::keccak256;
use secp256k1::{
    Message,
    ecdsa::{RecoverableSignature, RecoveryId},
};
use zkvm_interface::{execution::execution_program, io::JSONProgramInput};

sp1_zkvm::entrypoint!(main);

pub fn main() {
    println!("I'm in sp1 i guess?");

    let hex_str = "8778241e06de6f72eeada71c0f8e025d84a8a39fc4fa94796670a57cd81fa835000000000000000000000000000000000000000000000000000000000000001cc85353e6c551b2aa9846f8545256e6b9eef8bf3bd441dbabc865f5ffa64a8c7bb64f5e8164c0236b35e0609016e2e627027f89e913fe7826d49ffb3c3358e855";
    let bytes_vec = hex::decode(hex_str).expect("Invalid hex string");
    let calldata = Bytes::from(bytes_vec);

    println!("{:?}", calldata);

    let output = ecrecover(&calldata);

    println!("{:?}", output);

    // let input = sp1_zkvm::io::read::<JSONProgramInput>().0;
    // let output = execution_program(input).unwrap();

    // sp1_zkvm::io::commit(&output.encode());
}

pub fn ecrecover(calldata: &Bytes) -> Bytes {
    // Parse the input elements, first as a slice of bytes and then as an specific type of the crate
    let hash = calldata.get(0..32).unwrap();
    println!("Hash: {}", hex::encode(hash));
    let Ok(message) = Message::from_digest_slice(hash) else {
        println!("5");
        return Bytes::new();
    };

    println!("6");
    let v = u256_from_big_endian(calldata.get(32..64).unwrap());
    println!("v: {}", v);

    println!("7");
    // The Recovery identifier is expected to be 27 or 28, any other value is invalid
    if !(v == U256::from(27) || v == U256::from(28)) {
        println!("8");
        return Bytes::new();
    }

    println!("9");
    let v = u8::try_from(v).unwrap();
    let recovery_id_from_rpc = v.checked_sub(27).unwrap();
    let Ok(recovery_id) = RecoveryId::from_i32(recovery_id_from_rpc.into()) else {
        println!("10");
        return Bytes::new();
    };
    println!("Recovery Id: {:?}", recovery_id);

    println!("11");
    // signature is made up of the parameters r and s
    let sig = calldata.get(64..128).unwrap();
    let Ok(signature) = RecoverableSignature::from_compact(sig, recovery_id) else {
        println!("12");
        return Bytes::new();
    };

    // Important things
    println!("Signature: {:?}", signature);
    println!("Message: {:?}", message);

    println!("13");
    // Recover the address using secp256k1
    let Ok(public_key) = signature.recover(&message) else {
        println!("14");
        return Bytes::new();
    };

    println!("15");
    let mut public_key = public_key.serialize_uncompressed();

    println!("16");
    // We need to take the 64 bytes from the public key (discarding the first pos of the slice)
    keccak256(&mut public_key[1..65]);

    println!("17");
    // The output is 32 bytes: the initial 12 bytes with 0s, and the remaining 20 with the recovered address
    let mut output = vec![0u8; 12];
    output.extend_from_slice(public_key.get(13..33).unwrap());

    println!("18");
    Bytes::from(output.to_vec())
}
