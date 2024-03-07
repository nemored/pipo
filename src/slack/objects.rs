use std::fmt;

use serde::{
    de::{Error, Visitor},
    Deserialize, Deserializer, Serialize, Serializer,
};

#[derive(Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Disconnect {
        reason: String,
        debug_info: DebugInfo,
    },
    EventsApi {
        envelope_id: String,
        accepts_response_payload: bool,
        retry_attempt: u64,
        retry_reason: String,
        payload: EventPayload,
    },
    Hello {
        num_connections: u64,
        debug_info: DebugInfo,
        connection_info: ConnectionInfo,
    },
    SlashCommands {
        envelope_id: String,
        accepts_response_payload: bool,
        payload: SlashCommandPayload,
    },
    Unhandled {
        error: String,
    },
}

#[derive(Deserialize, Debug)]
pub struct ConnectionInfo {
    app_id: String,
}

#[derive(Deserialize, Debug)]
pub struct DebugInfo {
    host: String,
    build_number: Option<u64>,
    approximate_connection_time: Option<u64>,
}

#[derive(Deserialize, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventPayload {
    EventCallback {
        token: String,
        team_id: String,
        api_app_id: String,
        event: Event,
        event_id: String,
        event_time: i64,
        authorizations: Vec<Authorization>,
        is_ext_shared_channel: bool,
        event_context: String,
    },
    UrlVerification {
        token: String,
        challenge: String,
    },
}

#[derive(Deserialize, Debug)]
pub struct SlashCommandPayload {
    pub token: String,
    pub team_id: String,
    pub team_domain: String,
    pub channel_id: String,
    pub channel_name: String,
    pub user_id: String,
    pub user_name: String,
    pub command: String,
    pub text: String,
    pub api_app_id: String,
    pub is_enterprise_install: String,
    pub response_url: String,
    pub trigger_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Message {
    pub subtype: Option<String>,
    pub hidden: Option<bool>,
    pub message: Option<Box<Event>>,
    pub bot_id: Option<String>,
    pub client_msg_id: Option<String>,
    pub text: Option<String>,
    pub files: Option<Vec<File>>,
    pub upload: Option<bool>,
    pub user: Option<String>,
    pub display_as_bot: Option<bool>,
    pub ts: Option<String>,
    pub deleted_ts: Option<String>,
    pub team: Option<String>,
    pub attachments: Option<Vec<Attachment>>,
    pub blocks: Option<Vec<Block>>,
    pub channel: Option<String>,
    pub previous_message: Option<Box<Event>>,
    pub event_ts: Option<String>,
    pub thread_ts: Option<String>,
    pub channel_type: Option<String>,
    pub edited: Option<Edited>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Message(Message),
    PinAdded {
        channel_id: String,
        item: Box<Item>,
        pin_count: u64,
        pinned_info: PinnedInfo,
        event_ts: String,
        user: Option<String>,
    },
    PinRemoved {
        channel_id: String,
        item: Box<Item>,
        pin_count: u64,
        pinned_info: PinnedInfo,
        has_pins: bool,
        event_ts: String,
        user: Option<String>,
    },
    ReactionAdded {
        event_ts: Timestamp,
        item: Box<Event>,
        reaction: String,
        user: Option<String>,
        item_user: Option<String>,
    },
    ReactionRemoved {
        event_ts: Timestamp,
        item: Box<Event>,
        reaction: String,
        user: Option<String>,
        item_user: Option<String>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Item {
    Message {
        created: u64,
        created_by: String,
        channel: String,
        message: Event,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PinnedInfo {
    channel: String,
    pinned_ts: u64,
    pinned_by: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Edited {
    user: Option<String>,
    ts: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct File {
    pub id: String,
    pub created: u64,
    pub timestamp: u64,
    pub name: String,
    pub title: String,
    pub mimetype: String,
    pub filetype: String,
    pub pretty_type: String,
    pub user: String,
    pub editable: bool,
    pub size: u64,
    pub mode: String,
    pub is_external: bool,
    pub external_type: String,
    pub is_public: bool,
    pub public_url_shared: bool,
    pub display_as_bot: bool,
    pub username: String,
    pub url_private: String,
    pub url_private_download: String,
    pub thumb_64: String,
    pub thumb_80: String,
    pub thumb_360: String,
    pub thumb_360_w: u64,
    pub thumb_360_h: u64,
    pub thumb_480: Option<String>,
    pub thumb_480_w: u64,
    pub thumb_480_h: u64,
    pub thumb_160: String,
    pub thumb_720: String,
    pub thumb_720_w: u64,
    pub thumb_720_h: u64,
    pub thumb_800: Option<String>,
    pub thumb_800_w: u64,
    pub thumb_800_h: u64,
    pub original_w: u64,
    pub original_h: u64,
    pub thumb_tiny: String,
    pub permalink: String,
    pub permalink_public: String,
    pub has_rich_preview: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Attachment {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pretext: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_link: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts: Option<Timestamp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_link: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_subname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_width: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_height: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumb_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub footer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub footer_icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg_subtype: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_msg_unfurl: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mrkdwn_in: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_share: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Timestamp(pub String);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Actions {
        elements: Vec<Element>,
        block_id: Option<String>,
    },
    Context {
        elements: Vec<Element>,
        block_id: Option<String>,
    },
    Divider {
        block_id: Option<String>,
    },
    File {
        external_id: String,
        source: String,
        block_id: Option<String>,
    },
    Header {
        text: String,
        block_id: Option<String>,
    },
    Image {
        image_url: String,
        alt_text: String,
        title: Option<Text>,
        block_id: Option<String>,
    },
    Input {
        label: Text,
        element: Element,
        dispatch_action: Option<bool>,
        block_id: Option<String>,
        hint: Option<Text>,
        optional: Option<bool>,
    },
    RichText {
        elements: Vec<Element>,
        block_id: Option<String>,
    },
    Section {
        text: Option<Text>,
        block_id: Option<String>,
        fields: Option<Vec<Text>>,
        accessory: Option<Element>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Element {
    Button {
        text: Text,
        action_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        style: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        confirm: Option<ConfirmationDialog>,
    },
    Channel {
        channel_id: String,
    },
    ChannelsSelect {
        placeholder: Text,
        action_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        initial_channel: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        confirm: Option<ConfirmationDialog>,
        #[serde(skip_serializing_if = "Option::is_none")]
        response_url_enabled: Option<bool>,
    },
    Checkboxes {
        action_id: String,
        options: Vec<CompositionOption>,
        initial_options: Vec<CompositionOption>,
        #[serde(skip_serializing_if = "Option::is_none")]
        confirm: Option<ConfirmationDialog>,
    },
    ConversationsSelect {
        placeholder: Text,
        action_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        initial_conversation: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        default_to_current_conversation: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        confirm: Option<ConfirmationDialog>,
        #[serde(skip_serializing_if = "Option::is_none")]
        response_url_enabled: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        filter: Option<ConversationListFilter>,
    },
    Datepicker {
        action_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        placeholder: Option<Text>,
        #[serde(skip_serializing_if = "Option::is_none")]
        initial_date: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        confirm: Option<ConfirmationDialog>,
    },
    Emoji {
        name: String,
    },
    ExternalSelect {
        placeholder: Text,
        action_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        initial_option: Option<CompositionOption>,
        #[serde(skip_serializing_if = "Option::is_none")]
        min_query_length: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        confirm: Option<ConfirmationDialog>,
    },
    Image {
        image_url: String,
        alt_text: String,
    },
    Link {
        url: String,
    },
    MultiStaticSelect {
        placeholder: Text,
        action_id: String,
        options: Vec<CompositionOption>,
        #[serde(skip_serializing_if = "Option::is_none")]
        option_groups: Option<Vec<CompositionOptionGroup>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        initial_options: Option<Vec<CompositionOption>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        confirm: Option<ConfirmationDialog>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_selected_items: Option<i64>,
    },
    MultiExternalSelect {
        placeholder: Text,
        action_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        min_query_length: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        initial_options: Option<Vec<CompositionOption>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        confirm: Option<ConfirmationDialog>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_selected_items: Option<i64>,
    },
    MultiUsersSelect {
        placeholder: Text,
        action_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        initial_users: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        confirm: Option<ConfirmationDialog>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_selected_items: Option<i64>,
    },
    MultiConversationsSelect {
        placeholder: Text,
        action_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        initial_conversations: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        default_to_current_conversation: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        confirm: Option<ConfirmationDialog>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_selected_items: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        filter: Option<ConversationListFilter>,
    },
    MultiChannelsSelect {
        placeholder: Text,
        action_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        initial_channels: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        confirm: Option<ConfirmationDialog>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_selected_items: Option<i64>,
    },
    Overflow {
        action_id: String,
        options: Vec<CompositionOption>,
        #[serde(skip_serializing_if = "Option::is_none")]
        confirm: Option<ConfirmationDialog>,
    },
    PlainTextInput {
        action_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        placeholder: Option<Text>,
        #[serde(skip_serializing_if = "Option::is_none")]
        initial_value: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        multiline: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        min_length: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_length: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        dispatch_action_config: Option<DispatchActionConfiguration>,
    },
    RadioButtons {
        action_id: String,
        options: Vec<CompositionOption>,
        #[serde(skip_serializing_if = "Option::is_none")]
        initial_option: Option<CompositionOption>,
        #[serde(skip_serializing_if = "Option::is_none")]
        confirm: Option<ConfirmationDialog>,
    },
    RichTextList {
        elements: Vec<Element>,
        style: String,
        indent: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        border: Option<u64>,
    },
    RichTextPreformatted {
        elements: Vec<Element>,
    },
    RichTextQuote {
        elements: Vec<Element>,
    },
    RichTextSection {
        elements: Vec<Element>,
    },
    StaticSelect {
        placeholder: String,
        action_id: String,
        options: Vec<CompositionOption>,
        #[serde(skip_serializing_if = "Option::is_none")]
        option_groups: Option<Vec<CompositionOptionGroup>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        initial_options: Option<Vec<CompositionOption>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        confirm: Option<ConfirmationDialog>,
    },
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        style: Option<Style>,
    },
    Timepicker {
        action_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        placeholder: Option<Text>,
        #[serde(skip_serializing_if = "Option::is_none")]
        initial_time: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        confirm: Option<ConfirmationDialog>,
    },
    User {
        user_id: String,
    },
    UsersSelect {
        placeholder: Text,
        action_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        initial_user: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        confirm: Option<ConfirmationDialog>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Style {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strike: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Text {
    PlainText {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        emoji: Option<bool>,
    },
    Mrkdwn {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        verbatim: Option<bool>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConfirmationDialog {
    pub title: Text,
    pub text: Text,
    pub confirm: Text,
    pub deny: Text,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompositionOption {
    pub text: Text,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Text>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompositionOptionGroup {
    pub label: Text,
    pub options: Vec<CompositionOption>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DispatchActionConfiguration {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_actions_on: Option<Vec<String>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConversationListFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_external_shared_channels: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_bot_users: Option<bool>,
}

#[derive(Deserialize, Debug, Serialize)]
pub struct Authorization {
    enterprise_id: Option<String>,
    team_id: String,
    user_id: String,
    is_bot: bool,
    is_enterprise_install: bool,
}

#[derive(Deserialize, Debug)]
pub struct UsersList {
    pub ok: bool,
    pub error: Option<String>,
    pub cache_ts: Option<u64>,
    pub response_metadata: Option<ResponseMetadata>,
    pub members: Option<Vec<User>>,
}

#[derive(Deserialize, Debug)]
pub struct ResponseMetadata {
    pub next_cursor: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct User {
    pub id: Option<String>,
    pub team_id: Option<String>,
    pub name: Option<String>,
    pub deleted: Option<bool>,
    pub color: Option<String>,
    pub real_name: Option<String>,
    pub tz: Option<String>,
    pub tz_label: Option<String>,
    pub tz_offset: Option<i64>,
    pub profile: Option<Profile>,
    pub is_admin: Option<bool>,
    pub is_owner: Option<bool>,
    pub is_primary_owner: Option<bool>,
    pub is_restricted: Option<bool>,
    pub is_ultra_restricted: Option<bool>,
    pub is_bot: Option<bool>,
    pub is_stranger: Option<bool>,
    pub updated: Option<u64>,
    pub is_app_user: Option<bool>,
    pub is_invited_user: Option<bool>,
    pub has_2fa: Option<bool>,
    pub local: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct Profile {
    pub title: Option<String>,
    pub phone: Option<String>,
    pub skype: Option<String>,
    pub real_name: Option<String>,
    pub real_name_normalized: Option<String>,
    pub display_name: Option<String>,
    pub display_name_normalized: Option<String>,
    pub status_text: Option<String>,
    pub status_emoji: Option<String>,
    pub status_expiration: Option<i64>,
    pub avatar_hash: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub email: Option<String>,
    pub image_original: Option<String>,
    pub image_24: Option<String>,
    pub image_32: Option<String>,
    pub image_48: Option<String>,
    pub image_72: Option<String>,
    pub image_192: Option<String>,
    pub image_512: Option<String>,
    pub image_1024: Option<String>,
    pub team: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ChatPostMessage {
    pub channel: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_user: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<Attachment>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocks: Option<Vec<Block>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_annotation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_emoji: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_names: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mrkdwn: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_broadcast: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_ts: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unfurl_links: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unfurl_media: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ChatPostEphemeralMessage {
    pub channel: String,
    pub user: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_user: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<Attachment>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocks: Option<Vec<Block>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_emoji: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_names: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_ts: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ChatUpdate {
    pub channel: String,
    pub ts: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_user: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<Attachment>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocks: Option<Vec<Block>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_names: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_broadcast: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChatPostMessageResponse {
    pub ok: bool,
    pub error: Option<String>,
    pub channel: Option<String>,
    pub ts: Option<String>,
    pub message: Option<Event>,
}

struct StringVisitor;

impl<'de> Visitor<'de> for StringVisitor {
    type Value = String;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str(
            "a string, boolean, integer, or floating-point \
                 value",
        )
    }

    fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E>
    where
        E: Error,
    {
        Ok(format!("{}", v))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
    where
        E: Error,
    {
        Ok(format!("{}", v))
    }

    fn visit_i128<E>(self, v: i128) -> Result<Self::Value, E>
    where
        E: Error,
    {
        Ok(format!("{}", v))
    }

    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
    where
        E: Error,
    {
        Ok(format!("{}", v))
    }

    fn visit_u128<E>(self, v: u128) -> Result<Self::Value, E>
    where
        E: Error,
    {
        Ok(format!("{}", v))
    }

    fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
    where
        E: Error,
    {
        Ok(format!("{}", v))
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: Error,
    {
        Ok(String::from(v))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: Error,
    {
        Ok(String::from("null"))
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StringVisitor)
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StringVisitor).map(Timestamp)
    }
}

impl Serialize for Timestamp {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}
