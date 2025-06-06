//! Tables and data models.
//!
//! # Overview
//!
//! This module defines the tables in reth, as well as some table-related abstractions:
//!
//! - [`codecs`] integrates different codecs into [`Encode`] and [`Decode`]
//! - [`models`](crate::models) defines the values written to tables
//!
//! # Database Tour
//!
//! TODO(onbjerg): Find appropriate format for this...

mod raw;
use ethrex_common::{types::{BlockHash, BlockNumber, Index}, H256, U256};
use ethrex_common::types::Receipt;
pub use raw::{RawDupSort, RawKey, RawTable, RawValue, TableRawRow};

use crate::{
    error::StoreError,
    mdbx::table::{Decodable, DupSort, Encodable, Table, TableInfo},
    rlp::{AccountCodeHashRLP, AccountCodeRLP, BlockBodyRLP, BlockHashRLP, BlockHeaderRLP, Rlp, TupleRLP}, store_db::libmdbx::IndexedChunk,
};
use serde::{Deserialize, Serialize};
use std::fmt::{self, Debug};

/// Enum for the types of tables present in libmdbx.
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum TableType {
    /// key value table
    Table,
    /// Duplicate key value table
    DupSort,
}

/// The general purpose of this is to use with a combination of Tables enum,
/// by implementing a `TableViewer` trait you can operate on db tables in an abstract way.
///
/// # Example
///
/// ```
/// use reth_db_api::{
///     table::{DupSort, Table},
///     TableViewer, Tables,
/// };
///
/// struct MyTableViewer;
///
/// impl TableViewer<()> for MyTableViewer {
///     type Error = &'static str;
///
///     fn view<T: Table>(&self) -> Result<(), Self::Error> {
///         // operate on table in a generic way
///         Ok(())
///     }
///
///     fn view_dupsort<T: DupSort>(&self) -> Result<(), Self::Error> {
///         // operate on a dupsort table in a generic way
///         Ok(())
///     }
/// }
///
/// let viewer = MyTableViewer {};
///
/// let _ = Tables::Headers.view(&viewer);
/// let _ = Tables::Transactions.view(&viewer);
/// ```
pub trait TableViewer<R> {
    /// The error type returned by the viewer.
    type Error;

    /// Calls `view` with the correct table type.
    fn view_rt(&self, table: Tables) -> Result<R, Self::Error> {
        table.view(self)
    }

    /// Operate on the table in a generic way.
    fn view<T: Table>(&self) -> Result<R, Self::Error>;

    /// Operate on the dupsort table in a generic way.
    ///
    /// By default, the `view` function is invoked unless overridden.
    fn view_dupsort<T: DupSort>(&self) -> Result<R, Self::Error> {
        self.view::<T>()
    }
}

/// General trait for defining the set of tables
/// Used to initialize database
pub trait TableSet {
    /// Returns an iterator over the tables
    fn tables() -> Box<dyn Iterator<Item = Box<dyn TableInfo>>>;
}

/// Defines all the tables in the database.
#[macro_export]
macro_rules! tables {
    (@bool) => { false };
    (@bool $($t:tt)+) => { true };

    (@view $name:ident $v:ident) => { $v.view::<$name>() };
    (@view $name:ident $v:ident $_subkey:ty) => { $v.view_dupsort::<$name>() };

    (@value_doc $key:ty, $value:ty) => {
        concat!("[`", stringify!($value), "`]")
    };
    // Don't generate links if we have generics
    (@value_doc $key:ty, $value:ty, $($generic:ident),*) => {
        concat!("`", stringify!($value), "`")
    };

    ($($(#[$attr:meta])* table $name:ident$(<$($generic:ident $(= $default:ty)?),*>)? { type Key = $key:ty; type Value = $value:ty; $(type SubKey = $subkey:ty;)? } )*) => {
        // Table marker types.
        $(
            $(#[$attr])*
            ///
            #[doc = concat!("Marker type representing a database table mapping [`", stringify!($key), "`] to ", tables!(@value_doc $key, $value, $($($generic),*)?), ".")]
            $(
                #[doc = concat!("\n\nThis table's `DUPSORT` subkey is [`", stringify!($subkey), "`].")]
            )?
            pub struct $name$(<$($generic $( = $default)?),*>)? {
                _private: std::marker::PhantomData<($($($generic,)*)?)>,
            }

            // Ideally this implementation wouldn't exist, but it is necessary to derive `Debug`
            // when a type is generic over `T: Table`. See: https://github.com/rust-lang/rust/issues/26925
            impl$(<$($generic),*>)? fmt::Debug for $name$(<$($generic),*>)? {
                fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
                    unreachable!("this type cannot be instantiated")
                }
            }

            impl$(<$($generic),*>)? $crate::mdbx::table::Table for $name$(<$($generic),*>)?
            where
                $value: $crate::mdbx::table::Value + 'static
                $($(,$generic: Send + Sync)*)?
            {
                const NAME: &'static str = table_names::$name;
                const DUPSORT: bool = tables!(@bool $($subkey)?);

                type Key = $key;
                type Value = $value;
            }

            $(
                impl DupSort for $name {
                    type SubKey = $subkey;
                }
            )?
        )*

        // Tables enum.

        /// A table in the database.
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Tables {
            $(
                #[doc = concat!("The [`", stringify!($name), "`] database table.")]
                $name,
            )*
        }

        impl Tables {
            /// All the tables in the database.
            pub const ALL: &'static [Self] = &[$(Self::$name,)*];

            /// The number of tables in the database.
            pub const COUNT: usize = Self::ALL.len();

            /// Returns the name of the table as a string.
            pub const fn name(&self) -> &'static str {
                match self {
                    $(
                        Self::$name => table_names::$name,
                    )*
                }
            }

            /// Returns `true` if the table is a `DUPSORT` table.
            pub const fn is_dupsort(&self) -> bool {
                match self {
                    $(
                        Self::$name => tables!(@bool $($subkey)?),
                    )*
                }
            }

            /// The type of the given table in database.
            pub const fn table_type(&self) -> TableType {
                if self.is_dupsort() {
                    TableType::DupSort
                } else {
                    TableType::Table
                }
            }

            /// Allows to operate on specific table type
            pub fn view<T, R>(&self, visitor: &T) -> Result<R, T::Error>
            where
                T: ?Sized + TableViewer<R>,
            {
                match self {
                    $(
                        Self::$name => tables!(@view $name visitor $($subkey)?),
                    )*
                }
            }
        }

        impl fmt::Debug for Tables {
            #[inline]
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.name())
            }
        }

        impl fmt::Display for Tables {
            #[inline]
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.name().fmt(f)
            }
        }

        impl std::str::FromStr for Tables {
            type Err = String;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $(
                        table_names::$name => Ok(Self::$name),
                    )*
                    s => Err(format!("unknown table: {s:?}")),
                }
            }
        }

        impl TableInfo for Tables {
            fn name(&self) -> &'static str {
                self.name()
            }

            fn is_dupsort(&self) -> bool {
                self.is_dupsort()
            }
        }

        impl TableSet for Tables {
            fn tables() -> Box<dyn Iterator<Item = Box<dyn TableInfo>>> {
                Box::new(Self::ALL.iter().map(|table| Box::new(*table) as Box<dyn TableInfo>))
            }
        }

        // Need constants to match on in the `FromStr` implementation.
        #[expect(non_upper_case_globals)]
        mod table_names {
            $(
                pub(super) const $name: &'static str = stringify!($name);
            )*
        }

        /// Maps a run-time [`Tables`] enum value to its corresponding compile-time [`Table`] type.
        ///
        /// This is a simpler alternative to [`TableViewer`].
        ///
        /// # Examples
        ///
        /// ```
        /// use reth_db_api::{table::Table, Tables, tables_to_generic};
        ///
        /// let table = Tables::Headers;
        /// let result = tables_to_generic!(table, |GenericTable| <GenericTable as Table>::NAME);
        /// assert_eq!(result, table.name());
        /// ```
        #[macro_export]
        macro_rules! tables_to_generic {
            ($table:expr, |$generic_name:ident| $e:expr) => {
                match $table {
                    $(
                        Tables::$name => {
                            use $crate::tables::$name as $generic_name;
                            $e
                        },
                    )*
                }
            };
        }
    };
}

tables! {
    /// Stores the header hashes belonging to the canonical chain.
    table CanonicalBlockHashes {
        type Key = BlockNumber;
        type Value = BlockHashRLP;
    }

    table BlockNumbers {
        type Key = BlockHashRLP;
        type Value = BlockNumber;
    }

    table Headers {
        type Key = BlockHashRLP;
        type Value = BlockHeaderRLP;
    }

    table Bodies {
        type Key = BlockHashRLP;
        type Value = BlockBodyRLP;
    }

    table AccountCodes {
        type Key = AccountCodeHashRLP;
        type Value = AccountCodeRLP;
    }

    table Receipts {
        type Key = TupleRLP<BlockHash, Index>;
        type Value = IndexedChunk<Receipt>;
        type SubKey = Index;
    }
}

/// Keys for the `ChainState` table.
#[derive(Ord, Clone, Eq, PartialOrd, PartialEq, Debug, Deserialize, Serialize, Hash)]
pub enum ChainStateKey {
    /// Last finalized block key
    LastFinalizedBlock,
    /// Last finalized block key
    LastSafeBlockBlock,
}

impl Encodable for ChainStateKey {
    type Encoded = [u8; 1];

    fn encode(self) -> Self::Encoded {
        match self {
            Self::LastFinalizedBlock => [0],
            Self::LastSafeBlockBlock => [1],
        }
    }
}

impl Decodable for ChainStateKey {
    fn decode(value: &[u8]) -> Result<Self, StoreError> {
        match value {
            [0] => Ok(Self::LastFinalizedBlock),
            [1] => Ok(Self::LastSafeBlockBlock),
            _ => Err(StoreError::DecodeError),
        }
    }
}

impl Encodable for u64 {
    type Encoded = [u8; 4];

    fn encode(self) -> Self::Encoded {
        self.to_be_bytes()
    }
}

impl Decodable for u64 {
    fn decode(value: &[u8]) -> Result<Self, StoreError> {
        Ok(u64::from_be_bytes(&value))
    }
}

impl Encodable for H256 {
    type Encoded = [u8; 32];

    fn encode(self) -> Self::Encoded {
        self.0
    }
}

impl Decodable for H256 {
    fn decode(value: &[u8]) -> Result<Self, StoreError> {
        Ok(H256::from_slice(value))
    }
}

impl Encodable for U256 {
    type Encoded = [u8; 32];

    fn encode(self) -> Self::Encoded {
        self.to_big_endian()
    }
}

impl Decodable for U256 {
    fn decode(value: &[u8]) -> Result<Self, StoreError> {
        Ok(U256::from_big_endian(value))
    }
}

impl<T: Debug + Send + Sync> Decodable for Rlp<T> {
    fn decode(value: &[u8]) -> Result<Self, StoreError> {
        Ok(Self::from_bytes(value.to_vec()))
    }

    fn decode_owned(value: Vec<u8>) -> Result<Self, StoreError> {
        Ok(Self::from_bytes(value))
    }
}

impl<T: Debug + Send + Sync> Encodable for Rlp<T> {
    type Encoded = Vec<u8>;

    fn encode(self) -> Self::Encoded {
        self.bytes().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn parse_table_from_str() {
        for table in Tables::ALL {
            assert_eq!(format!("{table:?}"), table.name());
            assert_eq!(table.to_string(), table.name());
            assert_eq!(Tables::from_str(table.name()).unwrap(), *table);
        }
    }
}
