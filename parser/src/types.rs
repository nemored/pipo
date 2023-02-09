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

pub enum Element<'a> {
    Text {
        text: &'a str,
    },
    Span {
        style: Option<Style<'a>>,
        elements: Vec<Element<'a>>,
    },
    Paragraph {
        style: Style<'a>,
        elements: Vec<Element<'a>>,
    },
    Preformatted {
        style: Style<'a>,
        elements: Vec<Element<'a>>,
    },
    Bold {
        style: Style<'a>,
        elements: Vec<Element<'a>>,
    },
    Italic {
        style: Style<'a>,
        elements: Vec<Element<'a>>,
    },
    Underline {
        style: Style<'a>,
        elements: Vec<Element<'a>>,
    },
    Image {
        source: &'a str,
    },
    Reference {
        location: &'a str,
    },
}

pub struct Document<'a> {
    elements: Vec<Element<'a>>,
}

impl Document<'_> {
    pub fn new() -> Document<'static> {
        let elements = Vec::new();
        
        Document { elements }
    }

    pub fn push(&mut self, element: Element) {
        self.elements.push(element);
    }
}
