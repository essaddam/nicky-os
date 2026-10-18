//! One private, bounded consumer. Never serialize or broadcast these inputs.
use moli_core::DeviceId;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use tokio::sync::mpsc;

const CAPACITY: usize = 16;
/// A short-lived code; no Clone/Serialize and deliberately redacted Debug.
pub struct KeypadCode(String);
impl KeypadCode {
    #[must_use]
    pub fn new(code: String) -> Option<Self> {
        (code.len() <= 64 && !code.chars().any(char::is_control)).then_some(Self(code))
    }
    /// For the trusted consumer's verifier only, never for a public response.
    #[must_use]
    pub fn expose_for_verification(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for KeypadCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("KeypadCode(<redacted>)")
    }
}

/// The future alarm consumes this directly; the public bus sees only an audit.
pub struct KeypadInput {
    pub device: DeviceId,
    pub action: String,
    pub code: KeypadCode,
    pub transaction: Option<u64>,
}
impl fmt::Debug for KeypadInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("KeypadInput(<redacted>)")
    }
}

/// The receiver cannot be closed while kept alive: only dropping it permits
/// another consumer to register, and drops every pending code.
///
/// ```compile_fail
/// let hub = moli_runtime::Hub::new(Default::default()).unwrap();
/// let mut inputs = hub.take_keypad_inputs().unwrap();
/// inputs.close();
/// ```
#[derive(Debug)]
pub struct KeypadReceiver(mpsc::Receiver<KeypadInput>);
impl KeypadReceiver {
    pub async fn recv(&mut self) -> Option<KeypadInput> {
        self.0.recv().await
    }
    pub fn try_recv(&mut self) -> Result<KeypadInput, mpsc::error::TryRecvError> {
        self.0.try_recv()
    }
}

#[derive(Debug, Default)]
pub(crate) struct Inputs {
    sender: Mutex<Option<mpsc::Sender<KeypadInput>>>,
    rejected: AtomicU64,
}
impl Inputs {
    pub(crate) fn subscribe(&self) -> Option<KeypadReceiver> {
        let mut slot = self.sender.lock().unwrap_or_else(PoisonError::into_inner);
        if slot.as_ref().is_some_and(|sender| !sender.is_closed()) {
            return None;
        }
        let (sender, receiver) = mpsc::channel(CAPACITY);
        *slot = Some(sender);
        Some(KeypadReceiver(receiver))
    }
    pub(crate) fn send(&self, input: KeypadInput) -> bool {
        let accepted = self
            .sender
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_some_and(|sender| sender.try_send(input).is_ok());
        if !accepted {
            self.reject();
        }
        accepted
    }
    pub(crate) fn reject(&self) {
        self.rejected.fetch_add(1, Ordering::Relaxed);
    }
    pub(crate) fn rejected(&self) -> u64 {
        self.rejected.load(Ordering::Relaxed)
    }
}
