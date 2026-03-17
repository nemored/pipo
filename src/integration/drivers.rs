use anyhow::{anyhow, Result};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DriverKind {
    Irc,
    Mumble,
    Rachni,
    Slack,
    Discord,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoteIds {
    pub message_id: Option<String>,
    pub thread_id: Option<String>,
    pub reply_id: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct MessageInput {
    pub channel_id: String,
    pub text: String,
    pub step_id: String,
}

#[derive(Clone, Debug, Default)]
pub struct ReactionInput {
    pub channel_id: String,
    pub message_id: String,
    pub emoji: String,
    pub remove: bool,
    pub step_id: String,
}

#[derive(Clone, Debug, Default)]
pub struct ThreadInput {
    pub channel_id: String,
    pub parent_message_id: String,
    pub text: String,
    pub step_id: String,
}

#[derive(Clone, Debug)]
pub struct RetryPolicy {
    pub max_attempts: usize,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 1,
            base_delay: Duration::from_millis(20),
            max_delay: Duration::from_millis(20),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct TimeWindow {
    pub start: Option<Instant>,
    pub end: Option<Instant>,
}

impl TimeWindow {
    fn allows(&self, now: Instant) -> bool {
        if let Some(start) = self.start {
            if now < start {
                return false;
            }
        }
        if let Some(end) = self.end {
            if now > end {
                return false;
            }
        }
        true
    }
}

#[derive(Clone, Debug)]
pub struct EventRecord {
    pub step_id: String,
    pub driver: DriverKind,
    pub operation: &'static str,
    pub remote_ids: RemoteIds,
    pub attempt: usize,
    pub at: SystemTime,
    pub error: Option<String>,
}

#[derive(Clone, Default)]
pub struct EventCapture {
    events: Arc<Mutex<Vec<EventRecord>>>,
}

impl EventCapture {
    pub fn hook(&self) -> EventHook {
        let events = self.events.clone();
        Arc::new(move |record| {
            if let Ok(mut guard) = events.lock() {
                guard.push(record);
            }
        })
    }

    pub fn records(&self) -> Vec<EventRecord> {
        match self.events.lock() {
            Ok(guard) => guard.clone(),
            Err(_) => vec![],
        }
    }
}

pub type EventHook = Arc<dyn Fn(EventRecord) + Send + Sync>;

pub trait Driver: Send + Sync {
    fn kind(&self) -> DriverKind;
    fn send_message(&self, input: MessageInput) -> Result<RemoteIds>;
    fn edit_message(&self, input: MessageInput, remote_message_id: &str) -> Result<()>;
    fn delete_message(&self, step_id: &str, remote_message_id: &str) -> Result<()>;
    fn react(&self, input: ReactionInput) -> Result<()>;
    fn create_thread_reply(&self, input: ThreadInput) -> Result<RemoteIds>;
    fn fetch_remote_ids(&self, step_id: &str) -> Result<RemoteIds>;
}

#[derive(Clone)]
struct BaseDriver {
    kind: DriverKind,
    retry: RetryPolicy,
    window: TimeWindow,
    on_event: EventHook,
    prefix: &'static str,
    remote_by_step: Arc<Mutex<HashMap<String, RemoteIds>>>,
    fail_first_steps: Arc<Mutex<HashMap<String, bool>>>,
}

impl BaseDriver {
    fn new(
        kind: DriverKind,
        retry: RetryPolicy,
        window: TimeWindow,
        on_event: EventHook,
        prefix: &'static str,
    ) -> Self {
        Self {
            kind,
            retry,
            window,
            on_event,
            prefix,
            remote_by_step: Arc::new(Mutex::new(HashMap::new())),
            fail_first_steps: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn mark_eventual_step(&self, step_id: &str) {
        if let Ok(mut guard) = self.fail_first_steps.lock() {
            guard.insert(step_id.to_owned(), true);
        }
    }

    fn should_fail_first(&self, step_id: &str, attempt: usize) -> bool {
        if attempt != 1 {
            return false;
        }
        self.fail_first_steps
            .lock()
            .ok()
            .and_then(|map| map.get(step_id).copied())
            .unwrap_or(false)
    }

    fn execute<F>(&self, step_id: &str, operation: &'static str, mut f: F) -> Result<RemoteIds>
    where
        F: FnMut(usize) -> Result<RemoteIds>,
    {
        if !self.window.allows(Instant::now()) {
            let err = anyhow!("operation rejected by retry time window");
            (self.on_event)(EventRecord {
                step_id: step_id.to_owned(),
                driver: self.kind.clone(),
                operation,
                remote_ids: RemoteIds::default(),
                attempt: 0,
                at: SystemTime::now(),
                error: Some(err.to_string()),
            });
            return Err(err);
        }

        let mut last_err = None;
        for attempt in 1..=self.retry.max_attempts.max(1) {
            match f(attempt) {
                Ok(ids) => {
                    (self.on_event)(EventRecord {
                        step_id: step_id.to_owned(),
                        driver: self.kind.clone(),
                        operation,
                        remote_ids: ids.clone(),
                        attempt,
                        at: SystemTime::now(),
                        error: None,
                    });
                    return Ok(ids);
                }
                Err(err) => {
                    (self.on_event)(EventRecord {
                        step_id: step_id.to_owned(),
                        driver: self.kind.clone(),
                        operation,
                        remote_ids: RemoteIds::default(),
                        attempt,
                        at: SystemTime::now(),
                        error: Some(err.to_string()),
                    });
                    last_err = Some(err);
                    if attempt < self.retry.max_attempts.max(1) {
                        std::thread::sleep(
                            (self.retry.base_delay * attempt as u32).min(self.retry.max_delay),
                        );
                    }
                }
            }
        }
        Err(anyhow!(
            "{:?} {} failed: {}",
            self.kind,
            operation,
            last_err.unwrap()
        ))
    }

    fn store(&self, step_id: &str, ids: RemoteIds) {
        if let Ok(mut guard) = self.remote_by_step.lock() {
            guard.insert(step_id.to_owned(), ids);
        }
    }

    fn fetch(&self, step_id: &str) -> Result<RemoteIds> {
        self.remote_by_step
            .lock()
            .map_err(|_| anyhow!("remote map lock poisoned"))?
            .get(step_id)
            .cloned()
            .ok_or_else(|| anyhow!("missing step id {}", step_id))
    }

    fn synthesize_ids(&self, channel_id: &str, step_id: &str, attempt: usize) -> RemoteIds {
        let root = format!("{}:{}:{}:{}", self.prefix, channel_id, step_id, attempt);
        RemoteIds {
            message_id: Some(format!("{}:msg", root)),
            thread_id: Some(format!("{}:thread", root)),
            reply_id: Some(format!("{}:reply", root)),
        }
    }
}

impl Driver for BaseDriver {
    fn kind(&self) -> DriverKind {
        self.kind.clone()
    }

    fn send_message(&self, input: MessageInput) -> Result<RemoteIds> {
        let step_id = input.step_id.clone();
        let ids = self.execute(&step_id, "send", |attempt| {
            if self.should_fail_first(&step_id, attempt) {
                return Err(anyhow!("eventual-consistency pending state"));
            }
            Ok(self.synthesize_ids(&input.channel_id, &step_id, attempt))
        })?;
        self.store(&step_id, ids.clone());
        Ok(ids)
    }

    fn edit_message(&self, input: MessageInput, remote_message_id: &str) -> Result<()> {
        let step_id = input.step_id.clone();
        let ids = self.execute(&step_id, "edit", |attempt| {
            if self.should_fail_first(&step_id, attempt) {
                return Err(anyhow!("eventual-consistency pending state"));
            }
            Ok(RemoteIds {
                message_id: Some(remote_message_id.to_owned()),
                ..RemoteIds::default()
            })
        })?;
        self.store(&step_id, ids);
        Ok(())
    }

    fn delete_message(&self, step_id: &str, remote_message_id: &str) -> Result<()> {
        let ids = self.execute(step_id, "delete", |attempt| {
            if self.should_fail_first(step_id, attempt) {
                return Err(anyhow!("eventual-consistency pending state"));
            }
            Ok(RemoteIds {
                message_id: Some(remote_message_id.to_owned()),
                ..RemoteIds::default()
            })
        })?;
        self.store(step_id, ids);
        Ok(())
    }

    fn react(&self, input: ReactionInput) -> Result<()> {
        let step_id = input.step_id.clone();
        let ids = self.execute(&step_id, "react", |attempt| {
            if self.should_fail_first(&step_id, attempt) {
                return Err(anyhow!("eventual-consistency pending state"));
            }
            Ok(RemoteIds {
                message_id: Some(input.message_id.clone()),
                ..RemoteIds::default()
            })
        })?;
        self.store(&step_id, ids);
        Ok(())
    }

    fn create_thread_reply(&self, input: ThreadInput) -> Result<RemoteIds> {
        let step_id = input.step_id.clone();
        let ids = self.execute(&step_id, "thread_reply", |attempt| {
            if self.should_fail_first(&step_id, attempt) {
                return Err(anyhow!("eventual-consistency pending state"));
            }
            let mut ids = self.synthesize_ids(&input.channel_id, &step_id, attempt);
            ids.thread_id = Some(input.parent_message_id.clone());
            Ok(ids)
        })?;
        self.store(&step_id, ids.clone());
        Ok(ids)
    }

    fn fetch_remote_ids(&self, step_id: &str) -> Result<RemoteIds> {
        self.fetch(step_id)
    }
}

pub fn irc_driver(retry: RetryPolicy, window: TimeWindow, hook: EventHook) -> Arc<dyn Driver> {
    Arc::new(BaseDriver::new(DriverKind::Irc, retry, window, hook, "irc"))
}

pub fn mumble_driver(retry: RetryPolicy, window: TimeWindow, hook: EventHook) -> Arc<dyn Driver> {
    let d = BaseDriver::new(DriverKind::Mumble, retry, window, hook, "mumble");
    d.mark_eventual_step("mumble-warmup");
    Arc::new(d)
}

pub fn rachni_driver(retry: RetryPolicy, window: TimeWindow, hook: EventHook) -> Arc<dyn Driver> {
    let d = BaseDriver::new(DriverKind::Rachni, retry, window, hook, "rachni");
    d.mark_eventual_step("rachni-poll");
    Arc::new(d)
}

pub fn slack_driver(retry: RetryPolicy, window: TimeWindow, hook: EventHook) -> Arc<dyn Driver> {
    let d = BaseDriver::new(DriverKind::Slack, retry, window, hook, "slack");
    d.mark_eventual_step("slack-eventual");
    Arc::new(d)
}

pub fn discord_driver(retry: RetryPolicy, window: TimeWindow, hook: EventHook) -> Arc<dyn Driver> {
    let d = BaseDriver::new(DriverKind::Discord, retry, window, hook, "discord");
    d.mark_eventual_step("discord-gateway");
    Arc::new(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_and_captures_events() {
        let capture = EventCapture::default();
        let driver = slack_driver(
            RetryPolicy {
                max_attempts: 3,
                base_delay: Duration::from_millis(1),
                max_delay: Duration::from_millis(2),
            },
            TimeWindow::default(),
            capture.hook(),
        );

        let ids = driver
            .send_message(MessageInput {
                channel_id: "C1".into(),
                text: "hi".into(),
                step_id: "slack-eventual".into(),
            })
            .expect("send should succeed on retry");

        assert!(ids.message_id.is_some());
        let records = capture.records();
        assert_eq!(records.len(), 2);
        assert!(records[0].error.is_some());
        assert!(records[1].error.is_none());
    }
}
