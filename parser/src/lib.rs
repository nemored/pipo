mod types;

use types::{Element, Document, Span, Style};

pub enum ParseError {
    BadAttributes,
    UnclosedQuotes,
    UnclosedTag,
    UnopenedTag,
    UnsupportedTag,
}

type ParseResult<T> = Result<T, ParseError>;

fn parse_attributes(input: &str) -> ParseResult<Vec<(&str,Option<&str>)>> {
    let mut input = input;
    let mut ret = Vec::new();
    let mut offset = 0;
    let mut length = 0;
    let mut has_value = false;
    let mut in_quotes = false;
    let mut parameter;
    let mut value;

    for chr in input.chars() {
        offset += 1;
        match chr {
            '\"' => {
                in_quotes = !in_quotes;
            }
            '=' => {
                if !in_quotes {
                    if has_value {
                        return Err(ParseError::BadAttributes);
                    }
                    else {
                        has_value = true;
                        (parameter, input) = input.split_at(offset);
                        offset = 0;
                    }
                }
            },
            ' ' => {
                if !in_quotes {
                    if has_value {
                        (value, input) = input.split_at(offset);
                        offset = 0;
                        ret.push((parameter, Some(value)));
                    }
                    else {
                        (parameter, input) = input.split_at(offset);
                        offset = 0;
                        ret.push((parameter, None));
                    }
                    has_value = false;
                }
            },
            _ => ()
        }
    }

    if in_quotes { return Err(ParseError::UnclosedQuotes) }
    if has_value {
        ret.push((parameter, Some(input)));
    }
    else {
        ret.push((input, None));
    }

    Ok(ret)
}

fn parse_inner(input: &str) {
    
}

fn parse_tag<E: Element>(input: &str) -> ParseResult<E> {
    let (tag, attributes) = input.split_once(' ').unwrap_or((input, ""));

    match tag {
        "span" => {
            let mut ret = Span::new();

            for attribute in parse_attributes(attributes)? {
                match attribute {
                    (style, Some(styles)) => {
                        ret.style = Some(Style::parse_str(styles)?);
                    },
                    _ => (),
                }
            }

            Ok(ret)
        },
        _ => Err(ParseError::UnsupportedTag)
    }
}

pub fn parse_mumble_html(input: &str) -> ParseResult<Document> {
    let mut document = Document::new();
    let mut word = String::new();
    let mut in_tag = false;
    
    for chr in input.chars() {
        match chr {
            '<' => {
                if in_tag {
                    return Err(ParseError::UnclosedTag);
                }
                else {
                    parse_inner(&word);
                    in_tag = true;
                }
            },
            '>' => {
                if in_tag {
                    parse_tag(&word);
                    in_tag = false;
                }
                else {
                    return Err(ParseError::UnopenedTag);
                }
            },
            _ => word.push(chr)
        }
    }
    
    Ok(document)
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
}
