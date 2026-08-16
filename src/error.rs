use thiserror::Error;

#[derive(Debug, Error)]
pub enum PluginError {
    #[error("invalid pointer")]
    InvalidPointer,
    #[error("inaccesible memory")]
    InaccesibleMemory,
    #[error("unexpected: {0}")]
    Unexpected(Box<str>),
    #[error("installation error: {0}")]
    Install(Box<dyn std::error::Error + Send + Sync>),
    #[error("material error: {0}")]
    Material(MaterialError),
    #[error("os error: {0}")]
    Windows(#[from] windows::core::Error),
    #[error("error: {0}")]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

#[derive(Debug, Error)]
pub enum MaterialError {
    #[error("invalid material")]
    InvalidMaterial,
    #[error("unexpected instance")]
    UnexpectedInstance,
    #[error("variable '{0}' is not found in material '{1}'")]
    VariableNotFound(Box<str>, Box<str>),
    #[error("vector access out of bound, reading {0} but its length is {1}")]
    VectorAccessOutOfBound(usize, usize)
}