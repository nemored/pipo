use anyhow::anyhow;

mod objects;
use crate::slack::objects::*;

fn handle_asterisk(string: &str, idx: usize, elements: &mut Vec<Element>)
    -> usize {
    // Shadow idx to our next location.
    let mut idx = idx + 1;
    // Initialize counter for asterisks as 1 since we've already
    // encountered one for this function to be called.
    //
    // beg_idx and end_idx will specify the first and last character
    // of the str contained within the markdown formatting. We
    // initialize them to the current index, so that setting them
    // is a simple use of the accumulation operator with the index
    // of our for loop as an operand.
    let mut countup = 1;
    let mut countdn = 1;
    let mut delta = 0;
    let mut beg_idx = idx;
    let mut end_idx = idx;

    // Continue iterating through the str at the index subsequent
    // to the one at which this function was called.
    for i in idx .. string.len() {
    // We are only concerned with matching more '*'. We count
    // how many appear before the first non-asterisk character,
    // then continue through the string until we encounter
    // another sequence of asterisks corresponding in its length
    // to that of the first. If this sequence of asterisks does
    // not match in length, we subtract the difference from
    // beg_idx.
    if string[i] == '*' { countup += 1; }
    else {
        beg_idx = i;
        break
    }
    }

    countdn = countup;

    // Now we search the string in reverse for corresponding asterisks.
    for i in (beg_idx .. string.len()).rev() {
    if string[i] == '*' {
        countdn -= 1;

        if countdn == 0 {
        end_idx = string.len() - i;
        break
        }
    }
    else if countdn != countup {
        // We've reached a non-'*' before expected. Now we need to
        // back up beg_idx by the difference and call ourselves on
        // the str slice from beg_idx to end_idx.
        let delta = countup - countdn;
        
        beg_idx = delta;
        end_idx = string.len() - i + delta;
        handle_asterisk(string[beg_idx:end_idx], 0);

        if delta == 0 {
        delta = countup - countdn;
        end_idx = string.len() - i;
        }
    }
    }

    if countdn != 0 {
    if delta == 0 {
        match countup {
        1 => {
            elements.push(Message::Text {
            text: format!("*"),
            style: None,
            });
        },
        2 => {
            let prev_elements = elements;
            let mut elements = parse_markdown(string[beg_idx:],
                              Some(elements))?;
            
            if let Element::Text { text, style } = elements[0] {
            if style.is_none() {
                elements[0] = Element::Text {
                text: format!("**{}", text),
                style: None,
                }
            }
            }
            else {
            elements.insert(0, Element::Text {
                text: format!("**"),
                style: None,
            });
            }

            prev_elements.append(elements);
        },
        3 => {
            elements.push(Element::Text {
            text: format!("*"),
            style: Style {
                bold: Some(true),
                code: None,
                italic: None,
                strike: None,
            },
            });
        },
        _ => unreachable!()
        }
    }
    else {
        match delta {
        1 => {
            beg_
        },
        2 => ,
        3 => ,
        _ => unreachable!()
        }
    }
    }

    
    
    return parse_markdown(string[beg_idx:end_idx], Some(elements))
}

fn parse_markdown(string: &str, elements: Option<Element>)
    -> anyhow::Result<Element> {
    let mut elements = match elements {
    Some(e) => e,
    None => Vec::new()
    };
    
    for i in 0 .. string.len() {
    match string[i] {
        '*' => {
        
        handle_asterisk(string, i, &elements);
        },
        '_' => 0,
        '~' => 0,
        '`' => 0,
        '>' => 0,
        _ => handle_text(
    }
    }
}
