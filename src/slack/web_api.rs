struct WebApi {
    header: Header
}

struct Header {
    chat_post_message: HeaderMap<HeaderValue>,
    chat_update: HeaderMap<HeaderValue>,
}

enum Method {
    ChatPostMessage,
    ChatUpdate,
}

impl WebApi {
    fn new(token: &str) -> WebApi {
    let bearer = format!("Bearer {}", token);
    let header = Header {
        chat_post_message: {
        let headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE,
                   "application/json".parse().unwrap());
        headers.insert(header::AUTHORIZATION, bearer.parse().unwrap());

        headers
        },
        chat_update: {
        let headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE,
                   "application/json".parse().unwrap());
        headers.insert(header::AUTHORIZATION, bearer.parse().unwrap());

        headers
        }
    }

    WebApi { header }
    }

    fn get_headers(&self, method: Method) -> HeaderMap<HeaderValue> {
    match method {
        Method::ChatPostMessage { self.header.chat_post_message },
        Method::ChatUpdate { self.header.chat_update },
    }
    }

    pub async fn chat_post_message(&mut self,
                   parameters: objects::ChatPostMessage)
    -> anyhow::Result<ChatPostMessageResponse> {
    let body = match serde_json::to_value(parameters)?.as_str()
        .ok_or_else(|| anyhow!("{}:{}:{} JSON Value couldn't be \
                    represented as `&str`", file!(),
                   line!(), column!()))?;

    let mut request = self.http
        .request(Method::POST,
             "https://slack.com/api/chat.postMessage")
        .headers(self.get_headers(Method::ChatPostMessage))
        .request.body(body);
    
    let response: ChatPostMessageResponse
        = serde_json::from_str(request.send().await?.text().await?
                   .as_str())?;
    
    if response.ok { return Ok(ChatPostMessageResponse) }
    else {
        let error = match response.error {
        Some(e) => return Err(anyhow!("chat.postMessage failed with \
                           error: {}", e)),
        None => return Err(anyhow!("chat.postMessage failed"))
        }
    }
    }

    pub async fn chat_post_ephemeral_message(&mut self,
                         parameters:
                         objects::ChatPostEphemeralMessage)
    -> anyhow::Result<ChatPostEphemeralMessageResponse> {
    let body = match serde_json::to_value(parameters)?.as_str()
        .ok_or_else(|| anyhow!("{}:{}:{} JSON Value couldn't be \
                    represented as `&str`", file!(),
                   line!(), column!()))?;

    let mut request = self.http
        .request(Method::POST,
             "https://slack.com/api/chat.postEphemeralMessage")
        .headers(self.get_headers(Method::ChatPostEphemeralMessage))
        .request.body(body);
    
    let response: ChatPostEphemeralMessageResponse
        = serde_json::from_str(request.send().await?.text().await?
                   .as_str())?;
    
    if response.ok { return Ok(ChatPostEphemeralMessageResponse) }
    else {
        let error = match response.error {
        Some(e) => return Err(anyhow!("chat.postMessage failed with \
                           error: {}", e)),
        None => return Err(anyhow!("chat.postMessage failed"))
        }
    }
    }

    pub async fn chat_update(&mut self,
                 parameters: objects::ChatUpdate)
    -> anyhow::Result<ChatUpdateResponse> {
    let body = match serde_json::to_value(parameters)?.as_str()
        .ok_or_else(|| anyhow!("{}:{}:{} JSON Value couldn't be \
                    represented as `&str`", file!(),
                   line!(), column!()))?;

    let mut request = self.http
        .request(Method::POST,
             "https://slack.com/api/chat.update")
        .headers(self.get_headers(Method::ChatUpdate))
        .request.body(body);
    
    let response: ChatUpdateResponse
        = serde_json::from_str(request.send().await?.text().await?
                   .as_str())?;
    
    if response.ok { return Ok(ChatUpdateResponse) }
    else {
        let error = match response.error {
        Some(e) => return Err(anyhow!("chat.postMessage failed with \
                           error: {}", e)),
        None => return Err(anyhow!("chat.postMessage failed"))
        }
    }
    }
}

async fn chat_post_message(&mut self, channel: &str, transport: String,
              username: String, avatar_url: Option<String>,
              message: Option<String>,
              _attachments: Option<Vec<crate::Attachment>>)
    -> anyhow::Result<()> {
    lazy_static! {
    //static ref USERNAME_MATCH: Regex
    //= Regex::new(r#"(?=(?:^| )(\w.*)@)"#);
    // "[...]... disturbing" - Solra Bizna
    static ref USERNAME_MATCH: Regex
        = Regex::new(r#"@([A-Za-z0-9_ ]*)"#).unwrap();
    }

    let mut headers = HeaderMap::new();
    let icon_url: String;
    let basename = username;
    let username = format!("{} ({})", &basename, transport);
    let message = message.as_ref().map(|s| {
    USERNAME_MATCH.replace_all(s, |caps: &Captures| {
        let mut username = &caps[1];
        let mut remainder = String::new();
        let mut run_loop = true;
        let mut first_run = true;
        
        while run_loop {
        if !first_run {
            username = match username.rsplit_once(' ') {
            Some((left, right)) => {
                remainder.insert_str(0,
                         &format!(" {}", right));
                left
            },
            None => {
                run_loop = false;
                username
            }
            };
        }
        else { first_run = false }
        if username.len() > 0 {
            for (usern, user) in self.users.iter() {
            eprintln!("username: {}, usern: {}", username,
                  usern);
            if username == usern.as_str() {
                if let Some(id) = &user.id {
                return format!("<@{}>{}", id, remainder)
                }
                break
            }
            }
        }
        }
        
        caps[0].to_string()
    })
    });
    
    if let Some(avatar_url) = avatar_url {
    lazy_static!{
        static ref RE: Regex = Regex::new(
        r#"^(http[s]?://(?:.*/)+.*\.)(webp)(\?.*)?$"#
        ).unwrap();
    }

    if RE.is_match(&avatar_url) {
        icon_url = RE.replace_all(&avatar_url, |captures: &Captures|
                      -> String {
        if let Some(query) = captures.get(3) {
            format!("{}png{}",
                &captures[1],
                query.as_str())
        }
        else {
            format!("{}png", &captures[1])
        }
        }).to_string();
    }
    else {
        icon_url = format!("",
                   basename);
    }
    }
    else {
    icon_url = format!("", basename);
    }

    headers.insert(header::CONTENT_TYPE,
           "application/json".parse()?);
    headers.insert(header::AUTHORIZATION,
           ("Bearer ".to_owned() + &self.bot_token).parse()?);

    let body = serde_json::json!({
    "channel":channel,
    "text":message,
    "username": username,
    "icon_url": icon_url}).to_string();
    
    let response = self.http
    .request(Method::POST,
         "https://slack.com/api/chat.postMessage")
    .headers(headers)
    .body(body)
    .send().await?;
    let json: Value = serde_json::from_str(response.text().await?
                       .as_str())?;
    if json["ok"] == false {
    return Err(anyhow!("E: slack.rs:Slack::post_message(): {}",
               json["error"]))
    }

    Ok(())
}
}
