use std::{
    collections::HashMap,
    env,
    io
};

use hmac::{
    Hmac,
    Mac,
    NewMac
};
use nix::sys::socket;
use serde_json;
use sha2::Sha256;

use outer_cgi::IO;
use pipo::slack::objects::*;

const VERSION_NUMBER: &'static [u8] = b"v0";

fn handler(io: &mut dyn IO, env: HashMap<String,String>) -> io::Result<i32> {
    if env.get("REQUEST_METHOD").unwrap() != "POST" {
	return Err(io::Error::last_os_error())
    }
    if env.get("CONTENT_TYPE").unwrap() != "application/json" {
	return Err(io::Error::last_os_error())
    }
    let timestamp = env.get("HTTP_X_SLACK_REQUEST_TIMESTAMP")
	.ok_or_else(io::Error::last_os_error)?;
    let mut body = Vec::new();
    io.read_to_end(&mut body)?;
    // Verify signature
    let shared_secret = env::var("SLACK_SHARED_SECRET")
	.map_err(|_| io::Error::last_os_error())?;
    let mut hmac = Hmac::<Sha256>::new_from_slice(shared_secret.as_bytes())
	.map_err(|_| io::Error::last_os_error())?;
    hmac.update(&[&[VERSION_NUMBER, timestamp.as_bytes()].join(&b":"[..])[..],
		  &body[..]].join(&b":"[..]));
    hmac.verify(env.get("HTTP_X_SLACK_SIGNATURE")
		.ok_or_else(io::Error::last_os_error)?.as_bytes())
	.map_err(|_| io::Error::last_os_error())?;

    let payload: EventPayload = serde_json::from_slice(&body)
	.map_err(|_| io::Error::last_os_error())?;

    match payload {
	EventPayload::EventCallback {
	    token,
	    team_id,
	    api_app_id,
	    event,
	    event_id,
	    event_time,
	    authorizations,
	    is_ext_shared_channel,
	    event_context,
	} => {
	    writeln!(io, "HTTP 200 OK")?;
	    drop(io); // Send the response.

	    socket::send(env::var("PIPO_SOCKET")
			 .map_err(|_| io::Error::last_os_error())?.parse()
			 .map_err(|_| io::Error::last_os_error())?,
			 serde_json::to_string(&EventPayload::EventCallback {
			     token,
			     team_id,
			     api_app_id,
			     event,
			     event_id,
			     event_time,
			     authorizations,
			     is_ext_shared_channel,
			     event_context,
			 }).map_err(|_| io::Error::last_os_error())?
			 .as_bytes(),
			 socket::MsgFlags::empty())?;
	},
	EventPayload::UrlVerification {
	    token: _,
	    challenge,
	} => {
	    writeln!(io, "HTTP 200 OK")?;
	    writeln!(io, "Content-type: text/plain")?;
	    writeln!(io, "{}", challenge)?;
	    drop(io); // Send the response.
	},
    }

    return Ok(0) 
}

fn main() {
    outer_cgi::main(|_| {}, handler);
}
