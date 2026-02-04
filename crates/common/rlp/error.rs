use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum RLPDecodeError {
    #[error("Invalid RLP length{}", fmt_ctx(.0))]
    InvalidLength(Option<&'static str>),
    #[error("Malformed RLP data{}", fmt_ctx(.0))]
    MalformedData(Option<&'static str>),
    #[error("Malformed boolean: expected 0x80 or 0x01, got 0x{0:02x}")]
    MalformedBoolean(u8),
    #[error("Expected RLP string, got list{}", fmt_ctx(.0))]
    UnexpectedList(Option<&'static str>),
    #[error("Expected RLP list, got string{}", fmt_ctx(.0))]
    UnexpectedString(Option<&'static str>),
    #[error("Invalid compression: {0}")]
    InvalidCompression(#[from] snap::Error),
    #[error("Incompatible protocol: {0}")]
    IncompatibleProtocol(String),
    #[error("{0}")]
    Custom(String),
}

fn fmt_ctx(ctx: &Option<&'static str>) -> String {
    ctx.map(|c| format!(" decoding {c}")).unwrap_or_default()
}

impl RLPDecodeError {
    pub fn invalid_length() -> Self {
        Self::InvalidLength(None)
    }

    pub fn malformed_data() -> Self {
        Self::MalformedData(None)
    }

    pub fn malformed_boolean(got: u8) -> Self {
        Self::MalformedBoolean(got)
    }

    pub fn unexpected_list() -> Self {
        Self::UnexpectedList(None)
    }

    pub fn unexpected_string() -> Self {
        Self::UnexpectedString(None)
    }

    pub fn with_context(self, ctx: &'static str) -> Self {
        match self {
            Self::InvalidLength(_) => Self::InvalidLength(Some(ctx)),
            Self::MalformedData(_) => Self::MalformedData(Some(ctx)),
            Self::UnexpectedList(_) => Self::UnexpectedList(Some(ctx)),
            Self::UnexpectedString(_) => Self::UnexpectedString(Some(ctx)),
            other => other,
        }
    }
}

#[derive(Debug, Error)]
pub enum RLPEncodeError {
    #[error("Invalid compression: {0}")]
    InvalidCompression(#[from] snap::Error),
    #[error("{0}")]
    Custom(String),
}
