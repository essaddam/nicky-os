//! Small MQTT envelopes; media bytes and credentials never enter retained state.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub const ROOT: &str = "moli-probe/music-poc";
pub const MAX_CONTROL_BYTES: usize = 4096;

#[derive(Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub v: u8,
    pub id: String,
    pub session: String,
    pub epoch: u64,
    pub media_id: String,
    pub operation: String,
}

pub struct Admission {
    session: String,
    deadline: Instant,
    seen: VecDeque<String>,
}

impl Admission {
    pub fn new(session: String) -> Self {
        Self {
            session,
            deadline: Instant::now() + Duration::from_secs(10),
            seen: VecDeque::new(),
        }
    }
    pub fn accept(&mut self, bytes: &[u8], retained: bool) -> Result<Command> {
        ensure!(!retained, "retained commands are forbidden");
        ensure!(
            bytes.len() <= MAX_CONTROL_BYTES,
            "control envelope too large"
        );
        ensure!(Instant::now() < self.deadline, "session admission expired");
        let command: Command = serde_json::from_slice(bytes).context("invalid control envelope")?;
        ensure!(
            command.v == 1 && command.epoch == 1 && command.session == self.session,
            "stale session"
        );
        ensure!(
            command.operation == "decode-to-null",
            "physical output is not enabled"
        );
        ensure!(
            safe_id(&command.id) && safe_id(&command.media_id),
            "invalid control identity"
        );
        ensure!(!self.seen.contains(&command.id), "duplicate command");
        ensure!(self.seen.len() < 64, "session command quota reached");
        self.seen.push_back(command.id.clone());
        Ok(command)
    }
}

fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Command {
        Command {
            v: 1,
            id: "one".into(),
            session: "session".into(),
            epoch: 1,
            media_id: "media".into(),
            operation: "decode-to-null".into(),
        }
    }
    #[test]
    fn replay_retained_old_epoch_and_physical_operations_are_refused() {
        let bytes = serde_json::to_vec(&sample()).unwrap();
        let mut admission = Admission::new("session".into());
        assert!(admission.accept(&bytes, true).is_err());
        assert!(admission.accept(&bytes, false).is_ok());
        assert!(admission.accept(&bytes, false).is_err());
        let mut command = sample();
        command.id = "two".into();
        command.epoch = 2;
        assert!(
            admission
                .accept(&serde_json::to_vec(&command).unwrap(), false)
                .is_err()
        );
        command.epoch = 1;
        command.operation = "play".into();
        assert!(
            admission
                .accept(&serde_json::to_vec(&command).unwrap(), false)
                .is_err()
        );
        admission.deadline = Instant::now();
        assert!(admission.accept(&bytes, false).is_err());
    }
}
