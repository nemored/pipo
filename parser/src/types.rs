pub trait Element {
    
}

struct Unit {
    pixels: i32,
    percentage: i32,
    inches: i32,
}

struct FontFamily<'a> {
    font_family: &'a str,
}

struct Color {
    red: u8,
    green: u8,
    blue: u8
}

pub struct Style<'a> {
    margin_top: Unit,
    margin_bottom: Unit,
    margin_left: Unit,
    margin_right: Unit,
    qt_block_indent: i32,
    text_indent: Unit,
    font_family: FontFamily<'a>,
    color: Color,
}

struct Text<'a> {
    text: &'a str,
}

pub struct Span<'a> {
    style: Option<Style<'a>>,
    elements: Vec<&'a dyn Element>,
}

struct Paragraph<'a> {
    style: Style<'a>,
    elements: Vec<&'a dyn Element>,
}

struct Preformatted<'a> {
    style: Style<'a>,
    elements: Vec<&'a dyn Element>,
}

struct Bold<'a> {
    style: Style<'a>,
    elements: Vec<&'a dyn Element>,
}

struct Italic<'a> {
    style: Style<'a>,
    elements: Vec<&'a dyn Element>,
}

struct Underline<'a> {
    style: Style<'a>,
    elements: Vec<&'a dyn Element>,
}

struct Image<'a> {
    source: &'a str,
}

struct Reference<'a> {
    location: &'a str,
}

pub struct Document<'a> {
    elements: Vec<&'a dyn Element>,
}

impl Document<'_> {
    pub fn new() -> Document<'static> {
        let elements = Vec::new();
        
        Document { elements }
    }
}
