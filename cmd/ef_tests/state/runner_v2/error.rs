use std::path::PathBuf;

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
