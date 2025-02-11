use ethereum_types::Address;

use super::{Bytes48, Receipt};

pub type Bytes32 = [u8; 32];
pub type Bytes96 = [u8; 96];
const DEPOSIT_TYPE: u8 = 0x00;
const WITHDRAWAL_TYPE: u8 = 0x01;

lazy_static::lazy_static! {
    pub static ref DEPOSIT_CONTRACT_ADDRESS: Address = Address::from_slice(&hex::decode("00000000219ab540356cbb839cbe05303d7705fa").unwrap());
}

pub enum Request {
    Deposit(Deposit),
    Withdrawal,
}

impl Request {
    pub fn to_bytes(&self) -> [u8; 193] {
        let mut result = [0u8; 193];

        match self {
            Request::Deposit(d) => {
                result[0] = DEPOSIT_TYPE;
                let deposit_bytes = d.to_summarized_byte_array();
                result[1..].copy_from_slice(&deposit_bytes);
            }
            Request::Withdrawal => {
                result[0] = WITHDRAWAL_TYPE;
                // TODO: implement the withdrawal type
            }
        }
        result
    }
    pub fn from_deposits_receipts(receipts: &[Receipt]) -> Vec<Request> {
        let mut deposits = vec![];

        for r in receipts {
            for log in &r.logs {
                if log.address == *DEPOSIT_CONTRACT_ADDRESS {
                    let d = Deposit::from_abi_byte_array(&log.data);
                    // REMOVE LOG
                    // println!("Deposit: ");
                    // println!("Pub key: {:x?}", d.pub_key);
                    // println!("Withdrawal credentials: {:x?}", d.withdrawal_credentials);
                    // println!("Amount: {:?}", d.amount);
                    // println!("Signature: {:x?}", d.signature);
                    // println!("Index: {:?}", d.index);
                    // println!("--------------------------");
                    deposits.push(Self::Deposit(d));
                }
            }
        }
        deposits
    }
}

#[derive(Debug)]
pub struct Deposit {
    pub pub_key: Bytes48,
    pub withdrawal_credentials: Bytes32,
    pub amount: u64,
    pub signature: Bytes96,
    pub index: u64,
}

// Followed and ported implementation from:
// https://github.com/lightclient/go-ethereum/blob/5c4d46f3614d26654241849da7dfd46b95eed1c6/core/types/deposit.go#L61
impl Deposit {
    pub fn from_abi_byte_array(data: &[u8]) -> Deposit {
        // TODO: Handle errors
        if data.len() != 576 {
            panic!("Wrong data length");
        }

        //Encoding scheme:
        //
        //positional arguments -> 5 parameters with uint256 positional value for each -> 160b
        //pub_key: 32b of len + 48b padded to 64b
        //withdrawal_credentials: 32b of len + 32b
        //amount: 32b of len + 8b padded to 32b
        //signature: 32b of len + 96b
        //index: 32b of len + 8b padded to 32b
        //
        //-> Total len: 576 bytes

        let mut p = 32 * 5 + 32;

        let pub_key: Bytes48 = fixed_bytes_panic::<48>(data, p);
        p += 48 + 16 + 32;
        let withdrawal_credentials: Bytes32 = fixed_bytes_panic::<32>(data, p);
        p += 32 + 32;
        let amount: u64 = u64::from_le_bytes(fixed_bytes_panic::<8>(data, p));
        p += 8 + 24 + 32;
        let signature: Bytes96 = fixed_bytes_panic::<96>(data, p);
        p += 96 + 32;
        let index: u64 = u64::from_le_bytes(fixed_bytes_panic::<8>(data, p));

        Deposit {
            pub_key,
            withdrawal_credentials,
            amount,
            signature,
            index,
        }
    }

    pub fn to_summarized_byte_array(&self) -> [u8; 192] {
        let mut buffer = [0u8; 192];
        // pub_key + withdrawal_credentials + amount + signature + index
        let mut p = 0;
        buffer[p..48].clone_from_slice(&self.pub_key);
        p += 48;
        buffer[p..p + 32].clone_from_slice(&self.withdrawal_credentials);
        p += 32;
        buffer[p..p + 8].clone_from_slice(&self.amount.to_le_bytes());
        p += 8;
        buffer[p..p + 96].clone_from_slice(&self.signature);
        p += 96;
        buffer[p..p + 8].clone_from_slice(&self.index.to_le_bytes());

        buffer
    }
}

fn fixed_bytes_panic<const N: usize>(data: &[u8], offset: usize) -> [u8; N] {
    data.get(offset..offset + N)
        .expect("Couldn't convert to fixed bytes")
        .try_into()
        .expect("Couldn't convert to fixed bytes")
}
