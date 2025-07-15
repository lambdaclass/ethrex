use std::path::PathBuf;

#[derive(Debug)]
pub enum RunnerError {
    RootMismatch,
    FailedToGetAccountsUpdates,
    VMExecutionError(String),
    CurrentBaseFeeMissing,
    MaxPriorityFeePerGasMissing,
    MaxFeePerGasMissing,
    TxSucceededAndExceptionWasExpected,
    DifferentExceptionWasExpected,
    EIP7702ShouldNotBeCreateType,
    FailedToReadDirectory(PathBuf, String),
    FailedToConvertPath,
    FailedToGetFileType(String),
    FailedToParseTestFile(PathBuf, String),
    FailedToOpenFile(String),
    FailedToWriteReport(String),
    MissingJsonField(String),
    FailedToDeserializeField(String),
    FailedToCreateReportFile(String),
    FailedToGetIndexValue(String),
}
