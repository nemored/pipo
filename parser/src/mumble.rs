use std::{future::Pending, str::Chars};

use crate::{ParseResult, ParseError, Document, Element};

enum Attribute {
    Style {
        
    }
}

enum MumbleElement {
    Document {
        elements: Vec<MumbleElement>,
    },
    P {
        attributes: Vec<Attribute>,
        elements: Vec<MumbleElement>,
    },
    Pre {
        elements: Vec<MumbleElement>,
    },
    A {
        elements: Vec<MumbleElement>,
    },
    I {
        elements: Vec<MumbleElement>,
    },
    B {
        elements: Vec<MumbleElement>,
    },
    Span {
        elements: Vec<MumbleElement>,
    },
    Br,
    Img {
        source: ImgData,
    },
}

struct Mumble<'a> {
    input: &'a str,
    chars: Chars<'a>,
    cursor: usize,
    document: MumbleElement,
    cur_elem: &'a MumbleElement,
}

impl MumbleElement::Document {
    pub fn push(&mut self, element: MumbleElement) -> &MumbleElement {
        
    }
}

impl Parser for Mumble<'_> {
    fn parse(&mut self) -> ParseResult<Document> {
        let mut document = Document::new();

        for chr in input.chars() {
            
        }
   } 
}

impl<'a> Mumble<'a> {
    pub fn with_input(input: &'a str) -> Mumble<'a> {
        Mumble { input, cursor: 0 }
    }

    fn open_tag(&mut self) -> ParseResult<()> {
        let end = self.input.find(">").ok_or(ParseError::UnclosedTag)?;
        let element = match self.input.split_ascii_whitespace().next().ok_or(ParseError::MalformedTag)? {
            "p" => MumbleElement::P::new(),
            "pre" => MumbleElement::Pre::new(),
            "a" => MumbleElement::A::new(),
            "i" => MumbleElement::I::new(),
            "b" => MumbleElement::B::new(),
            "span" => MumbleElement::Span::new(),
            "br" => MumbleElement::Br,
            "img" => MumbleElement::Img::new(),
        };

        self.document.push(element);

        Ok(())
    }

    fn parse(&mut self) -> ParseResult<Vec<MumbleElement>> {
        while self.cursor < self.input.len() {
            match &self.input[cursor..cursor+1] {
                "<" => {
                    self.cursor += 1;
                    self.open_tag(); 
                }
            }
        }

        for chr in self.input.chars() {
            match chr {
                '<' => {
                    self.open_tag();
                },
                
            }
        }
    }

    fn parse_inner(&mut self) -> ParseResult<Vec<MumbleElement>> {
        let mut elements = Vec::new();
        let mut cur_tag = None;

        while self.cursor < self.input.len() {
            match &self.input[self.cursor..self.cursor+1] {
                "<" => {
                    if let Some(tag) = cur_tag {
                        
                    }
                    else {
                        let end = self.input[self.cursor+2..].find('>').ok_or(ParseError::UnclosedTag)?;
                        let (tag, attributes) = self.input[self.cursor+1..end]
                            .split_once(' ')
                            .unwrap_or_else((&self.input[self.cursor+1..end], ""));
                        self.cursor = end+1;
                        elements.push(match tag {
                            "p" => MumbleElement::P { attributes: self.parse_attributes(attributes), elements: self.parse_inner()? },
                        });
                    }
                },
                " " => {
                    
                }
            }
        }

        Ok(elements)
    }
}
