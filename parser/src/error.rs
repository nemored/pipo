pub enum ParseError {
    MalformedAttributes,
    MalformedTag,
    UnclosedQuotes,
    UnclosedTag,
    UnopenedTag,
    UnsupportedTag,
}

pub type ParseResult<T> = Result<T, ParseError>;
