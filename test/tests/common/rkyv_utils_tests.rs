use ethereum_types::{H160, H256};
use rkyv::{Archive, Deserialize, Serialize, rancor::Error};

use ethrex_common::types::AccessListItem;

#[test]
fn serialize_deserialize_acess_list() {
    #[derive(Deserialize, Serialize, Archive, PartialEq, Debug)]
    struct AccessListStruct {
        #[rkyv(with = ethrex_common::rkyv_utils::AccessListItemWrapper)]
        list: AccessListItem,
    }

    let address = H160::random();
    let key_list = (0..10).map(|_| H256::random()).collect::<Vec<_>>();
    let access_list = AccessListStruct {
        list: (address, key_list),
    };
    let bytes = rkyv::to_bytes::<Error>(&access_list).unwrap();
    let deserialized = rkyv::from_bytes::<AccessListStruct, Error>(bytes.as_slice()).unwrap();
    assert_eq!(access_list, deserialized)
}
