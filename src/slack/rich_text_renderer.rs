use anyhow::Result;
use async_recursion::async_recursion;
use serenity::async_trait;

use super::objects::{Element, Style};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RichTextNode {
    Section(Vec<RichTextNode>),
    PlainText(String),
    StyleSpan {
        bold: bool,
        italic: bool,
        strike: bool,
        children: Vec<RichTextNode>,
    },
    LineBreak,
    List {
        ordered: bool,
        indent: u64,
        items: Vec<RichTextNode>,
    },
    QuoteBlock(Vec<RichTextNode>),
    CodeSpan(String),
    CodeBlock(Vec<RichTextNode>),
    Mention(Mention),
    Link {
        url: String,
        text: Option<String>,
    },
    Emoji(String),
    Date(String),
    Unsupported(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Mention {
    User(String),
    Channel(String),
    Broadcast(String),
    Team(String),
    Usergroup(String),
}

#[async_trait]
pub trait RichTextResolver {
    async fn resolve_user_display_name(&mut self, user_id: &str) -> Result<String>;
    fn resolve_channel_name(&self, channel_id: &str) -> Option<String>;
}

pub fn build_ir(element: &Element) -> RichTextNode {
    match element {
        Element::RichTextSection { elements } => RichTextNode::Section(
            elements
                .iter()
                .flat_map(build_inline_nodes)
                .collect::<Vec<_>>(),
        ),
        Element::RichTextList {
            elements,
            style,
            indent,
            border: _,
        } => RichTextNode::List {
            ordered: style == "ordered",
            indent: *indent,
            items: elements.iter().map(build_ir).collect(),
        },
        Element::RichTextPreformatted { elements } => {
            RichTextNode::CodeBlock(elements.iter().map(build_ir).collect())
        }
        Element::RichTextQuote { elements } => {
            RichTextNode::QuoteBlock(elements.iter().map(build_ir).collect())
        }
        _ => {
            let mut nodes = build_inline_nodes(element);
            if nodes.len() == 1 {
                nodes.remove(0)
            } else {
                RichTextNode::Section(nodes)
            }
        }
    }
}

fn build_inline_nodes(element: &Element) -> Vec<RichTextNode> {
    match element {
        Element::Text { text, style } => styled_text_to_nodes(text, style.as_ref()),
        Element::Emoji { name } => vec![RichTextNode::Emoji(name.clone())],
        Element::Link { url, text, .. } => vec![RichTextNode::Link {
            url: url.clone(),
            text: text.clone(),
        }],
        Element::User { user_id } => vec![RichTextNode::Mention(Mention::User(user_id.clone()))],
        Element::Channel { channel_id } => {
            vec![RichTextNode::Mention(Mention::Channel(channel_id.clone()))]
        }
        Element::Broadcast { range } => {
            vec![RichTextNode::Mention(Mention::Broadcast(range.clone()))]
        }
        Element::Date { fallback, .. } => vec![RichTextNode::Date(fallback.clone())],
        Element::Team { team_id } => vec![RichTextNode::Mention(Mention::Team(team_id.clone()))],
        Element::Usergroup { usergroup_id } => {
            vec![RichTextNode::Mention(Mention::Usergroup(
                usergroup_id.clone(),
            ))]
        }
        Element::RichTextSection { .. }
        | Element::RichTextList { .. }
        | Element::RichTextPreformatted { .. }
        | Element::RichTextQuote { .. } => vec![build_ir(element)],
        _ => vec![RichTextNode::Unsupported(format!(
            "unsupported element: {element:?}"
        ))],
    }
}

fn styled_text_to_nodes(text: &str, style: Option<&Style>) -> Vec<RichTextNode> {
    if text.is_empty() {
        return vec![RichTextNode::PlainText(String::new())];
    }

    let Some(style) = style else {
        return split_text_with_line_breaks(text);
    };

    let mut chars = text.chars().peekable();
    let mut leading = String::new();
    while matches!(chars.peek(), Some(' ')) {
        leading.push(chars.next().unwrap());
    }

    let mut trailing = String::new();
    let mut core = chars.collect::<String>();
    while core.ends_with(' ') {
        trailing.push(' ');
        core.pop();
    }
    trailing = trailing.chars().rev().collect::<String>();

    let mut nodes = Vec::new();
    if !leading.is_empty() {
        nodes.push(RichTextNode::PlainText(leading));
    }

    let bold = style.bold.unwrap_or(false);
    let italic = style.italic.unwrap_or(false);
    let strike = style.strike.unwrap_or(false);
    let code = style.code.unwrap_or(false);

    let core_nodes = split_text_with_line_breaks(&core);
    if !core_nodes.is_empty() {
        let content = if strike {
            RichTextNode::StyleSpan {
                bold: false,
                italic: false,
                strike: true,
                children: core_nodes,
            }
        } else if code {
            RichTextNode::CodeSpan(core)
        } else if bold || italic {
            RichTextNode::StyleSpan {
                bold,
                italic,
                strike: false,
                children: core_nodes,
            }
        } else {
            RichTextNode::Section(core_nodes)
        };

        nodes.push(content);
    }

    if !trailing.is_empty() {
        nodes.push(RichTextNode::PlainText(trailing));
    }

    nodes
}

fn split_text_with_line_breaks(text: &str) -> Vec<RichTextNode> {
    let mut nodes = Vec::new();

    for (idx, part) in text.split('\n').enumerate() {
        if idx > 0 {
            nodes.push(RichTextNode::LineBreak);
        }
        if !part.is_empty() {
            nodes.push(RichTextNode::PlainText(part.to_string()));
        }
    }

    nodes
}

pub async fn render<R>(resolver: &mut R, element: &Element) -> Result<String>
where
    R: RichTextResolver + Send,
{
    render_node(resolver, &build_ir(element)).await
}

#[async_recursion]
pub async fn render_node<R>(resolver: &mut R, node: &RichTextNode) -> Result<String>
where
    R: RichTextResolver + Send,
{
    match node {
        RichTextNode::Section(children) => render_children(resolver, children).await,
        RichTextNode::PlainText(text) => Ok(text.clone()),
        RichTextNode::StyleSpan {
            bold,
            italic,
            strike,
            children,
        } => {
            let body = render_children(resolver, children).await?;
            if *strike {
                return Ok(format!("~~{}~~", body));
            }

            let mut prefix = String::new();
            let mut suffix = String::new();
            if *bold {
                prefix.push_str("**");
                suffix = format!("**{}", suffix);
            }
            if *italic {
                prefix.push('*');
                suffix = format!("*{}", suffix);
            }

            Ok(format!("{}{}{}", prefix, body, suffix))
        }
        RichTextNode::LineBreak => Ok("\n".to_string()),
        RichTextNode::List {
            ordered,
            indent,
            items,
        } => {
            let mut rendered = String::new();
            let indent_str = "\t".repeat(*indent as usize);
            for (idx, item) in items.iter().enumerate() {
                let bullet = if *ordered {
                    format!("{}.", idx + 1)
                } else {
                    "*".to_string()
                };
                rendered.push_str(&format!(
                    "{}{} {}\n",
                    indent_str,
                    bullet,
                    render_node(resolver, item).await?
                ));
            }
            Ok(rendered)
        }
        RichTextNode::QuoteBlock(children) => Ok(format!(
            ">>> {}",
            render_children(resolver, children).await?
        )),
        RichTextNode::CodeSpan(text) => Ok(format!("`{}`", text)),
        RichTextNode::CodeBlock(children) => Ok(format!(
            "```{}```",
            render_children(resolver, children).await?
        )),
        RichTextNode::Mention(Mention::User(user_id)) => Ok(format!(
            "@{}",
            resolver.resolve_user_display_name(user_id).await?
        )),
        RichTextNode::Mention(Mention::Channel(channel_id)) => {
            let channel = resolver
                .resolve_channel_name(channel_id)
                .unwrap_or_else(|| "Unknown".to_string());
            Ok(format!("#{}", channel))
        }
        RichTextNode::Mention(Mention::Broadcast(range)) => Ok(format!("<!{}>", range)),
        RichTextNode::Mention(Mention::Team(team_id)) => Ok(format!("<!team^{}>", team_id)),
        RichTextNode::Mention(Mention::Usergroup(usergroup_id)) => {
            Ok(format!("<!subteam^{}>", usergroup_id))
        }
        RichTextNode::Link { url, text } => Ok(text.clone().unwrap_or_else(|| url.clone())),
        RichTextNode::Emoji(name) => Ok(format!(":{}:", name)),
        RichTextNode::Date(fallback) => Ok(fallback.clone()),
        RichTextNode::Unsupported(desc) => Ok(format!("[{}]", desc)),
    }
}

async fn render_children<R>(resolver: &mut R, children: &[RichTextNode]) -> Result<String>
where
    R: RichTextResolver + Send,
{
    let mut rendered = String::new();
    for child in children {
        rendered.push_str(&render_node(resolver, child).await?);
    }
    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestResolver;

    #[async_trait]
    impl RichTextResolver for TestResolver {
        async fn resolve_user_display_name(&mut self, user_id: &str) -> Result<String> {
            Ok(format!("user-{user_id}"))
        }

        fn resolve_channel_name(&self, channel_id: &str) -> Option<String> {
            Some(format!("chan-{channel_id}"))
        }
    }

    #[tokio::test]
    async fn renders_lists_and_mentions() {
        let element = Element::RichTextList {
            elements: vec![Element::RichTextSection {
                elements: vec![Element::User {
                    user_id: "U123".to_string(),
                }],
            }],
            style: "ordered".to_string(),
            indent: 1,
            border: None,
        };

        let mut resolver = TestResolver;
        let rendered = render(&mut resolver, &element).await.unwrap();
        assert_eq!(rendered, "\t1. @user-U123\n");
    }

    #[tokio::test]
    async fn unsupported_elements_use_fallback() {
        let element = Element::Button {
            text: super::super::objects::Text::PlainText {
                text: "x".to_string(),
                emoji: None,
            },
            action_id: "a".to_string(),
            url: None,
            value: None,
            style: None,
            confirm: None,
        };

        let mut resolver = TestResolver;
        let rendered = render(&mut resolver, &element).await.unwrap();
        assert!(rendered.contains("unsupported element"));
    }
}
