# Provers dependencies

The project includes dependencies from KZG and secp256k1 library.

| Module  | secp256k1   | KZG      |
|---------|-------------|----------|
| L1      | `secp256k1` | `c-kzg`  |
| SP1     | `k256`      | `kzg-rs` |
| Risc0   | XXXXXX      | xxxxx    |


`secp256k1` is more optimal than `k256` (5x more in Ggas/s).
