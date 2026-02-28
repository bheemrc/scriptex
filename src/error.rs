use thiserror::Error;

#[derive(Error, Debug)]
pub enum SonicError {
    #[error("Lexer error at position {pos}: {msg}")]
    LexError { pos: usize, msg: String },

    #[error("Parse error at token {token_idx}: {msg}")]
    ParseError { token_idx: usize, msg: String },

    #[error("Layout error: {0}")]
    LayoutError(String),

    #[error("PDF generation error: {0}")]
    PdfError(String),

    #[error("Font error: {0}")]
    FontError(String),

    #[error("Unknown command: \\{0}")]
    UnknownCommand(String),

    #[error("Missing argument for \\{0}")]
    MissingArgument(String),

    #[error("Environment mismatch: expected \\end{{{expected}}}, got \\end{{{got}}}")]
    EnvironmentMismatch { expected: String, got: String },

    #[error("Unclosed group")]
    UnclosedGroup,

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

pub type SonicResult<T> = Result<T, SonicError>;
