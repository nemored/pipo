use anyhow::Result;
use async_recursion::async_recursion;
use serenity::async_trait;

use super::{
    entity_resolver::{
        format_broadcast, format_channel, format_link, format_team, format_user, format_usergroup,
    },
    objects::{Element, Style},
};

const IRC_BOLD: char = '\u{0002}';
const IRC_ITALIC: char = '\u{001d}';
const IRC_MONOSPACE: char = '\u{0011}';
const IRC_STRIKETHROUGH: char = '\u{001e}';
const IRC_RESET: char = '\u{000f}';

const IRC_STYLE_STACK: &[(IrcStyle, char)] = &[
    (IrcStyle::Bold, IRC_BOLD),
    (IrcStyle::Italic, IRC_ITALIC),
    (IrcStyle::Strikethrough, IRC_STRIKETHROUGH),
    (IrcStyle::Monospace, IRC_MONOSPACE),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IrcStyle {
    Bold,
    Italic,
    Strikethrough,
    Monospace,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenderOptions {
    pub irc_formatting_enabled: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            irc_formatting_enabled: true,
        }
    }
}

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

pub async fn render<R>(
    resolver: &mut R,
    element: &Element,
    options: RenderOptions,
) -> Result<String>
where
    R: RichTextResolver + Send,
{
    render_node(resolver, &build_ir(element), options).await
}

fn split_lines_preserving_empty(input: &str) -> Vec<String> {
    input.split('\n').map(|line| line.to_string()).collect()
}

fn trim_trailing_empty_lines(lines: &mut Vec<String>) {
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
}

#[async_recursion]
pub async fn render_node<R>(
    resolver: &mut R,
    node: &RichTextNode,
    options: RenderOptions,
) -> Result<String>
where
    R: RichTextResolver + Send,
{
    match node {
        RichTextNode::Section(children) => render_section(resolver, children, options).await,
        RichTextNode::PlainText(text) => Ok(sanitize_irc_conflicts(text)),
        RichTextNode::StyleSpan {
            bold,
            italic,
            strike,
            children,
        } => {
            let body = render_children(resolver, children, options).await?;
            let styles = [
                (*bold, IrcStyle::Bold),
                (*italic, IrcStyle::Italic),
                (*strike, IrcStyle::Strikethrough),
            ];
            Ok(apply_irc_styles(&body, &styles, options))
        }
        RichTextNode::LineBreak => Ok("\n".to_string()),
        RichTextNode::List {
            ordered,
            indent,
            items,
        } => {
            let mut lines = Vec::new();
            let indent_str = "  ".repeat(*indent as usize);
            for (idx, item) in items.iter().enumerate() {
                let bullet = if *ordered {
                    format!("{}.", idx + 1)
                } else {
                    "-".to_string()
                };

                let mut item_lines =
                    split_lines_preserving_empty(&render_node(resolver, item, options).await?);
                trim_trailing_empty_lines(&mut item_lines);
                if item_lines.is_empty() {
                    item_lines.push(String::new());
                }

                let first_prefix = format!("{}{} ", indent_str, bullet);
                let continuation_prefix = " ".repeat(first_prefix.chars().count());

                lines.push(format!("{first_prefix}{}", item_lines[0]));
                for line in item_lines.iter().skip(1) {
                    lines.push(format!("{continuation_prefix}{line}"));
                }
            }
            Ok(lines.join("\n"))
        }
        RichTextNode::QuoteBlock(children) => {
            let mut lines =
                split_lines_preserving_empty(&render_children(resolver, children, options).await?);
            trim_trailing_empty_lines(&mut lines);
            if lines.is_empty() {
                lines.push(String::new());
            }
            Ok(lines
                .into_iter()
                .map(|line| format!("> {line}"))
                .collect::<Vec<_>>()
                .join("\n"))
        }
        RichTextNode::CodeSpan(text) => Ok(apply_irc_styles(
            &sanitize_irc_conflicts(text),
            &[(true, IrcStyle::Monospace)],
            options,
        )),
        RichTextNode::CodeBlock(children) => {
            let mut raw_options = options;
            raw_options.irc_formatting_enabled = false;
            let body = render_children(resolver, children, raw_options).await?;
            Ok(format!("```\n{}\n```", sanitize_irc_conflicts(&body)))
        }
        RichTextNode::Mention(Mention::User(user_id)) => {
            let user_name = resolver.resolve_user_display_name(user_id).await.ok();
            Ok(format_user(user_name.as_deref(), user_id))
        }
        RichTextNode::Mention(Mention::Channel(channel_id)) => {
            let channel_name = resolver.resolve_channel_name(channel_id);
            Ok(format_channel(channel_name.as_deref(), channel_id))
        }
        RichTextNode::Mention(Mention::Broadcast(range)) => Ok(format_broadcast(range)),
        RichTextNode::Mention(Mention::Team(team_id)) => Ok(format_team(None, team_id)),
        RichTextNode::Mention(Mention::Usergroup(usergroup_id)) => {
            Ok(format_usergroup(None, usergroup_id))
        }
        RichTextNode::Link { url, text } => {
            Ok(sanitize_irc_conflicts(&format_link(url, text.as_deref())))
        }
        RichTextNode::Emoji(name) => Ok(format!(":{}:", name)),
        RichTextNode::Date(fallback) => Ok(sanitize_irc_conflicts(fallback)),
        RichTextNode::Unsupported(desc) => Ok(format!("[{}]", sanitize_irc_conflicts(desc))),
    }
}

fn apply_irc_styles(body: &str, styles: &[(bool, IrcStyle)], options: RenderOptions) -> String {
    if body.is_empty() {
        return String::new();
    }

    if !options.irc_formatting_enabled {
        return body.to_string();
    }

    let mut prefix = String::new();
    for (style, code) in IRC_STYLE_STACK {
        if styles
            .iter()
            .any(|(enabled, current)| *enabled && current == style)
        {
            prefix.push(*code);
        }
    }

    if prefix.is_empty() {
        body.to_string()
    } else {
        format!("{prefix}{body}{IRC_RESET}")
    }
}

fn sanitize_irc_conflicts(input: &str) -> String {
    input
        .chars()
        .filter(|ch| {
            *ch != IRC_BOLD
                && *ch != IRC_ITALIC
                && *ch != IRC_MONOSPACE
                && *ch != IRC_STRIKETHROUGH
                && *ch != IRC_RESET
        })
        .collect()
}

async fn render_children<R>(
    resolver: &mut R,
    children: &[RichTextNode],
    options: RenderOptions,
) -> Result<String>
where
    R: RichTextResolver + Send,
{
    let mut rendered = String::new();
    for child in children {
        rendered.push_str(&render_node(resolver, child, options).await?);
    }
    Ok(rendered)
}

#[async_recursion]
async fn render_section<R>(
    resolver: &mut R,
    children: &[RichTextNode],
    options: RenderOptions,
) -> Result<String>
where
    R: RichTextResolver + Send,
{
    let mut rendered = String::new();
    let mut needs_separator = false;

    for child in children {
        match child {
            RichTextNode::List { .. }
            | RichTextNode::QuoteBlock(..)
            | RichTextNode::CodeBlock(..) => {
                if !rendered.is_empty() && !rendered.ends_with('\n') {
                    rendered.push('\n');
                }
                rendered.push_str(&render_node(resolver, child, options).await?);
                needs_separator = true;
            }
            RichTextNode::LineBreak => {
                rendered.push('\n');
                needs_separator = false;
            }
            _ => {
                if needs_separator && !rendered.ends_with('\n') {
                    rendered.push('\n');
                }
                rendered.push_str(&render_node(resolver, child, options).await?);
                needs_separator = false;
            }
        }
    }

    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::HashMap, fs, path::Path};

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
        let rendered = render(&mut resolver, &element, RenderOptions::default())
            .await
            .unwrap();
        assert_eq!(rendered, "  1. @user-U123");
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
        let rendered = render(&mut resolver, &element, RenderOptions::default())
            .await
            .unwrap();
        assert!(rendered.contains("unsupported element"));
    }

    #[tokio::test]
    async fn renders_irc_style_codes() {
        let element = Element::Text {
            text: " hi ".to_string(),
            style: Some(Style {
                bold: Some(true),
                italic: Some(true),
                strike: Some(false),
                code: Some(false),
            }),
        };

        let mut resolver = TestResolver;
        let rendered = render(&mut resolver, &element, RenderOptions::default())
            .await
            .unwrap();
        assert_eq!(rendered, " \u{0002}\u{001d}hi\u{000f} ");
    }

    #[tokio::test]
    async fn strips_embedded_irc_control_codes_from_plaintext() {
        let element = Element::Text {
            text: format!("bad{}text{}", '\u{0002}', '\u{001d}'),
            style: None,
        };

        let mut resolver = TestResolver;
        let rendered = render(&mut resolver, &element, RenderOptions::default())
            .await
            .unwrap();
        assert_eq!(rendered, "badtext");
    }
    #[tokio::test]
    async fn can_disable_irc_style_codes() {
        let element = Element::Text {
            text: " hi ".to_string(),
            style: Some(Style {
                bold: Some(true),
                italic: Some(true),
                strike: Some(false),
                code: Some(false),
            }),
        };

        let mut resolver = TestResolver;
        let rendered = render(
            &mut resolver,
            &element,
            RenderOptions {
                irc_formatting_enabled: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(rendered, " hi ");
    }

    #[tokio::test]
    async fn renders_link_label_with_url() {
        let element = Element::Link {
            url: "https://example.com".to_string(),
            text: Some("Example".to_string()),
            style: None,
        };

        let mut resolver = TestResolver;
        let rendered = render(&mut resolver, &element, RenderOptions::default())
            .await
            .unwrap();
        assert_eq!(rendered, "Example (https://example.com)");
    }

    #[tokio::test]
    async fn renders_unknown_mentions_deterministically() {
        struct UnknownResolver;

        #[async_trait]
        impl RichTextResolver for UnknownResolver {
            async fn resolve_user_display_name(&mut self, _user_id: &str) -> Result<String> {
                Err(anyhow::anyhow!("not found"))
            }

            fn resolve_channel_name(&self, _channel_id: &str) -> Option<String> {
                None
            }
        }

        let element = Element::RichTextSection {
            elements: vec![
                Element::User {
                    user_id: "U404".to_string(),
                },
                Element::Text {
                    text: " ".to_string(),
                    style: None,
                },
                Element::Channel {
                    channel_id: "C404".to_string(),
                },
            ],
        };

        let mut resolver = UnknownResolver;
        let rendered = render(&mut resolver, &element, RenderOptions::default())
            .await
            .unwrap();
        assert_eq!(rendered, "@unknown-user:U404 #unknown-channel:C404");
    }

    fn conformance_fixture_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/slack_rich_text")
    }

    fn parse_matrix_rows() -> HashMap<String, (String, String)> {
        let matrix_path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/slack-richtext-compat.md");
        let matrix =
            fs::read_to_string(matrix_path).expect("compatibility matrix doc should exist");
        matrix
            .lines()
            .filter(|line| line.starts_with('|') && !line.contains("---"))
            .filter_map(|line| {
                let cols = line.split('|').map(|col| col.trim()).collect::<Vec<_>>();
                if cols.len() < 6 {
                    return None;
                }
                let fixture = cols[1].trim_matches('`').to_string();
                if fixture == "Fixture" {
                    return None;
                }

                Some((fixture, (cols[3].to_string(), cols[4].to_string())))
            })
            .collect()
    }

    #[tokio::test]
    async fn slack_rich_text_conformance_suite_matches_goldens() {
        let fixtures_dir = conformance_fixture_dir();
        let mut fixtures = fs::read_dir(&fixtures_dir)
            .expect("fixture directory should exist")
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
                    Some(path)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        fixtures.sort();

        assert!(
            !fixtures.is_empty(),
            "conformance fixtures should not be empty"
        );

        for json_path in fixtures {
            let fixture = fs::read_to_string(&json_path).expect("fixture json should be readable");
            let element: Element = serde_json::from_str(&fixture)
                .expect("fixture json should deserialize into Element");

            let golden_path = json_path.with_extension("golden");
            assert!(
                golden_path.exists(),
                "missing golden output for fixture {}",
                json_path.display()
            );
            let expected = fs::read_to_string(&golden_path)
                .expect("golden fixture should be readable")
                .trim_end_matches('\n')
                .to_string();

            let mut resolver = TestResolver;
            let rendered = render(&mut resolver, &element, RenderOptions::default())
                .await
                .expect("fixture should render");
            assert_eq!(
                rendered,
                expected,
                "render mismatch for fixture {}",
                json_path.display()
            );
        }
    }

    #[test]
    fn compatibility_matrix_covers_conformance_fixtures() {
        let fixtures_dir = conformance_fixture_dir();
        let fixtures = fs::read_dir(&fixtures_dir)
            .expect("fixture directory should exist")
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let path = entry.path();
                (path.extension().and_then(|ext| ext.to_str()) == Some("json")).then_some(path)
            })
            .collect::<Vec<_>>();

        let matrix_rows = parse_matrix_rows();

        for fixture_path in fixtures {
            let fixture_name = fixture_path
                .file_stem()
                .and_then(|name| name.to_str())
                .expect("fixture name should be valid utf8")
                .to_string();

            let (status, justification) = matrix_rows
                .get(&fixture_name)
                .unwrap_or_else(|| panic!("missing matrix row for fixture `{fixture_name}`"));

            assert!(
                ["full", "partial", "unsupported"].contains(&status.as_str()),
                "invalid status `{status}` for fixture `{fixture_name}`"
            );

            if status != "full" {
                assert!(
                    !justification.is_empty() && justification != "N/A",
                    "fixture `{fixture_name}` is `{status}` but missing justification in docs/slack-richtext-compat.md"
                );
            }
        }
    }

    #[tokio::test]
    async fn renders_nested_layouts_with_golden_fixtures() {
        let list_in_quote = Element::RichTextQuote {
            elements: vec![Element::RichTextSection {
                elements: vec![
                    Element::Text {
                        text: "before".to_string(),
                        style: None,
                    },
                    Element::Text {
                        text: "\n".to_string(),
                        style: None,
                    },
                    Element::RichTextList {
                        elements: vec![
                            Element::RichTextSection {
                                elements: vec![Element::Text {
                                    text: "alpha".to_string(),
                                    style: None,
                                }],
                            },
                            Element::RichTextSection {
                                elements: vec![Element::Text {
                                    text: "beta".to_string(),
                                    style: None,
                                }],
                            },
                        ],
                        style: "bullet".to_string(),
                        indent: 0,
                        border: None,
                    },
                ],
            }],
        };

        let quote_in_list = Element::RichTextList {
            elements: vec![Element::RichTextSection {
                elements: vec![
                    Element::Text {
                        text: "item".to_string(),
                        style: None,
                    },
                    Element::Text {
                        text: "\n".to_string(),
                        style: None,
                    },
                    Element::RichTextQuote {
                        elements: vec![Element::RichTextSection {
                            elements: vec![
                                Element::Text {
                                    text: "q1".to_string(),
                                    style: None,
                                },
                                Element::Text {
                                    text: "\nq2".to_string(),
                                    style: None,
                                },
                            ],
                        }],
                    },
                ],
            }],
            style: "ordered".to_string(),
            indent: 0,
            border: None,
        };

        let code_within_list_item = Element::RichTextList {
            elements: vec![Element::RichTextSection {
                elements: vec![
                    Element::Text {
                        text: "run".to_string(),
                        style: None,
                    },
                    Element::Text {
                        text: "\n".to_string(),
                        style: None,
                    },
                    Element::RichTextPreformatted {
                        elements: vec![Element::RichTextSection {
                            elements: vec![Element::Text {
                                text: "echo hi\necho bye".to_string(),
                                style: None,
                            }],
                        }],
                    },
                ],
            }],
            style: "bullet".to_string(),
            indent: 0,
            border: None,
        };

        let mut resolver = TestResolver;
        let rendered_list_in_quote =
            render(&mut resolver, &list_in_quote, RenderOptions::default())
                .await
                .unwrap();
        assert_eq!(
            rendered_list_in_quote,
            include_str!("../../fixtures/slack_rich_text/list_in_quote.golden")
                .trim_end_matches('\n')
        );

        let mut resolver = TestResolver;
        let rendered_quote_in_list =
            render(&mut resolver, &quote_in_list, RenderOptions::default())
                .await
                .unwrap();
        assert_eq!(
            rendered_quote_in_list,
            include_str!("../../fixtures/slack_rich_text/quote_in_list.golden")
                .trim_end_matches('\n')
        );

        let mut resolver = TestResolver;
        let rendered_code_in_list = render(
            &mut resolver,
            &code_within_list_item,
            RenderOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(
            rendered_code_in_list,
            include_str!("../../fixtures/slack_rich_text/code_within_list.golden")
                .trim_end_matches('\n')
        );
    }
}
