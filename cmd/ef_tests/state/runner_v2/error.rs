use std::{collections::HashMap, path::PathBuf};

use ethrex_common::{Address, U256};
use bytes::Bytes;
use keccak_hash::H256;


#[derive(Debug, Clone)]
pub enum RunnerError {
    FailedToGetAccountsUpdates,
    CurrentBaseFeeMissing,
    MaxPriorityFeePerGasMissing,
    MaxFeePerGasMissing,
    EIP7702ShouldNotBeCreateType,
    FailedToReadDirectory(PathBuf, String),
    FailedToConvertPath,
    FailedToGetFileType(String),
    FailedToParseTestFile(PathBuf, String),
    FailedToOpenFile(String),
    FailedToWriteReport(String),
    FailedToCreateReportFile(String),
    FailedToGetIndexValue(String),
}


