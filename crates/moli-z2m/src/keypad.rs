//! Only the trusted internal channel may receive a code. No raw payload diagnostics.
use crate::Entry;
use moli_core::privacy;
use moli_runtime::{DriverCtx, KeypadCode, KeypadInput};
use serde_json::{Map, Value as Json};

pub(crate) fn hand_over(
    ctx: &DriverCtx,
    entry: &Entry,
    payload: &Map<String, Json>,
    retained: bool,
) {
    if !payload
        .iter()
        .any(|(key, value)| privacy::code_property(key) || privacy::private_json(value))
    {
        return;
    }
    let input = (!retained).then(|| decode(entry, payload)).flatten();
    match input {
        Some(input) => {
            ctx.submit_keypad_input(input);
        }
        None => ctx.reject_keypad_input(),
    }
}
fn decode(entry: &Entry, payload: &Map<String, Json>) -> Option<KeypadInput> {
    let action = payload.get("action")?.as_str()?;
    if action.is_empty() || action.len() > 64 || action.chars().any(char::is_control) {
        return None;
    }
    let mut fields = payload
        .iter()
        .filter(|(key, _)| privacy::code_property(key));
    let (_, raw) = fields.next()?;
    // Ambiguous credentials are rejected, never picked according to JSON key order.
    if fields.next().is_some() {
        return None;
    }
    let code = match raw {
        Json::String(text) => text.clone(),
        Json::Number(number) => number.as_u64()?.to_string(),
        _ => return None,
    };
    let transaction = match payload.get("action_transaction") {
        None | Some(Json::Null) => None,
        Some(value) => Some(value.as_u64()?),
    };
    Some(KeypadInput {
        device: entry.device.id.clone(),
        action: action.to_owned(),
        code: KeypadCode::new(code)?,
        transaction,
    })
}
