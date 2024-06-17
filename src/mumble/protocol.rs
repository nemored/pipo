enum RejectType {
    None,
    WrongVersion,
    InvalidUsername,
    WrongUserPW,
    WrongServerPW,
    UsernameInUse,
    ServerFull,
    NoCertificate,
    AuthenticatorFail,
}

enum DenyType {
    Text,
    Permission,
    SuperUser,
    ChannelName,
    TextTooLong,
    H9K,
    TemporaryChannel,
    MissingCertificate,
    UserName,
    ChannelFull,
    NestingLimit,
    ChannelCountLimit,
    ChannelListenerLimit,
    UserListenerLimit,
}

enum Context {
    Server,
    Channel,
    User = 4,
}

enum Operation {
    Add,
    Remove,
}

struct BanEntry {
    address: Vec<u8>,
    mask: u32,
    name: Option<String>,
    hash: Option<String>,
    reason: Option<String>,
    start: Option<String>,
    duration: Option<u32>,
}

struct ChanGroup {
    name: String,
    inherited: Option<bool>,
    inherit: Option<bool>,
    inheritable: Option<bool>,
    add: Vec<u32>,
    remove: Vec<u32>,
    inherited_members: Vec<u32>,
}

struct ChanACL {
    apply_here: Option<bool>,
    apply_subs: Option<bool>,
    inherited: Option<bool>,
    user_id: Option<u32>,
    group: Option<String>,
    grant: Option<u32>,
    deny: Option<u32>,
}

struct User {
    user_id: u32,
    name: Option<String>,
    last_seen: Option<String>,
    last_channel: Option<u32>,
}

struct Target {
    session: Vec<u32>,
    channel_id: Option<u32>,
    group: Option<String>,
    links: Option<bool>,
    children: Option<bool>,
}

struct Stats {
    good: Option<u32>,
    late: Option<u32>,
    lost: Option<u32>,
    resync: Option<u32>,
}

struct Version {
    version: Option<u32>,
    release: Option<String>,
    os: Option<String>,
    os_version: Option<String>,
}

struct UDPTunnel {
    packet: Vec<u8>,
}

struct Authenticate {
    username: Option<String>,
    password: Option<String>,
    tokens: Vec<String>,
    celt_versions: Vec<i32>,
    opus: Option<bool>,
}

struct Ping {
    timestamp: Option<u64>,
    good: Option<u32>,
    late: Option<u32>,
    lost: Option<u32>,
    resync: Option<u32>,
    udp_packets: Option<u32>,
    tcp_packets: Option<u32>,
    udp_ping_avg: Option<f32>,
    udp_ping_var: Option<f32>,
    tcp_ping_avg: Option<f32>,
    tcp_ping_var: Option<f32>,
}

struct Reject {
    type_: Option<RejectType>,
    reason: Option<String>,
}

struct ServerSync {
    session: Option<u32>,
    max_bandwidth: Option<u32>,
    welcome_text: Option<String>,
    permissions: Option<u64>,
}

struct ChannelRemove {
    channel_id: u32,
}

struct ChannelState {
    channel_id: Option<u32>,
    parent: Option<u32>,
    name: Option<String>,
    links: Vec<u32>,
    description: Option<String>,
    links_add: Vec<u32>,
    links_remove: Vec<u32>,
    temporary: Option<bool>,
    position: Option<i32>,
    description_hash: Option<Vec<u8>>,
    max_users: Option<u32>,
    is_enter_restricted: Option<bool>,
    can_enter: Option<bool>,
}

struct UserRemove {
    session: u32,
    actor: Option<u32>,
    reason: Option<String>,
    ban: Option<bool>,
}

struct UserState {
    session: Option<u32>,
    actor: Option<u32>,
    name: Option<String>,
    user_id: Option<u32>,
    channel_id: Option<u32>,
    mute: Option<bool>,
    deaf: Option<bool>,
    suppress: Option<bool>,
    self_mute: Option<bool>,
    self_deaf: Option<bool>,
    texture: Option<Vec<u8>>,
    plugin_context: Option<Vec<u8>>,
    plugin_identity: Option<String>,
    comment: Option<String>,
    hash: Option<String>,
    comment_hash: Option<String>,
    texture_hash: Option<String>,
    priority_speaker: Option<bool>,
    recording: Option<bool>,
    temporary_access_tokens: Vec<String>,
    listening_channel_add: Vec<u32>,
    listening_channel_remove: Vec<u32>,
}

struct BanList {
    bans: Vec<BanEntry>,
    query: Option<bool>,
}

struct TextMessage {
    actor: Option<u32>,
    session: Vec<u32>,
    channel_id: Vec<u32>,
    tree_id: Vec<u32>,
    message: Vec<String>,
}

struct PermissionDenied {
    permission: Option<u32>,
    channel_id: Option<u32>,
    session: Option<u32>,
    reason: Option<String>,
    type_: Option<DenyType>,
    name: Option<String>,
}

struct ACL {
    channel_id: u32,
    inherit_acls: Option<bool>,
    groups: Vec<ChanGroup>,
    acls: Vec<ChanACL>,
    query: Option<bool>,
}

struct QueryUsers {
    ids: Vec<u32>,
    names: Vec<String>,
}

struct CryptSetup {
    key: Option<Vec<u8>>,
    client_nonce: Option<Vec<u8>>,
    server_none: Option<Vec<u8>>,
}

struct ContextActionModify {
    action: String,
    text: Option<String>,
    context: Option<Context>,
    operation: Option<Operation>,
}

struct ContextAction {
    session: Option<u32>,
    channel_id: Option<u32>,
    action: String,
}

struct UserList {
    users: Vec<User>,
}

struct VoiceTarget {
    id: Option<u32>,
    targets: Vec<Target>,
}

struct PermissionQuery {
    channel_id: Option<u32>,
    permissions: Option<u32>,
    flush: Option<bool>,
}

struct CodecVersion {
    alpha: i32,
    beta: i32,
    prefer_alpha: bool,
    opus: Option<bool>,
}

struct UserStats {
    session: Option<u32>,
    stats_only: Option<bool>,
    certificates: Vec<u8>,
    from_client: Option<Stats>,
    from_server: Option<Stats>,
    udp_packets: Option<u32>,
    tcp_packets: Option<u32>,
    udp_ping_avg: Option<f32>,
    udp_ping_var: Option<f32>,
    tcp_ping_avg: Option<f32>,
    tcp_ping_var: Option<f32>,
    version: Option<Version>,
    celt_versions: Vec<i32>,
    address: Option<u8>,
    bandwidth: Option<u32>,
    onlinesecs: Option<u32>,
    idlesecs: Option<u32>,
    strong_certificate: Option<bool>,
    opus: Option<bool>,
}

struct RequestBlob {
    session_texture: Vec<u32>,
    session_comment: Vec<u32>,
    channel_description: Vec<u32>,
}

struct ServerConfig {
    max_bandwidth: Option<u32>,
    welcome_text: Option<String>,
    allow_html: Option<bool>,
    message_length: Option<u32>,
    image_message_length: Option<u32>,
    max_users: Option<u32>,
    recording_allowed: Option<bool>,
}

struct SuggestConfig {
    version: Option<u32>,
    positional: Option<bool>,
    push_to_talk: Option<bool>,
}

struct PluginDataTransmission {
    sender_session: Option<u32>,
    receiver_sessions: Vec<u32>,
    data: Option<Vec<u8>>,
    data_id: Option<String>,
}

enum Packet {
    Version(Version),
    UDPTunnel(UDPTunnel),
    Authenticate(Authenticate),
    Ping(Ping),
    Reject(Reject),
    ServerSync(ServerSync),
    ChannelRemove(ChannelRemove),
    ChannelState(ChannelState),
    UserRemove(UserRemove),
    UserState(UserState),
    BanList(BanList),
    TextMessage(TextMessage),
    PermissionDenied(PermissionDenied),
    ACL(ACL),
    QueryUsers(QueryUsers),
    CryptSetup(CryptSetup),
    ContextActionModify(ContextActionModify),
    ContextAction(ContextAction),
    UserList(UserList),
    VoiceTarget(VoiceTarget),
    PermissionQuery(PermissionQuery),
    CodecVersion(CodecVersion),
    UserStats(UserStats),
    RequestBlob(RequestBlob),
    ServerConfig(ServerConfig),
    SuggestConfig(SuggestConfig),
    PluginDataTransmission(PluginDataTransmission),
}
