//! A replacement queue, never an accumulating authentication history.
use crate::Failure;
use std::collections::VecDeque;
#[derive(Default)]
pub(crate) struct Tokens {
    values: VecDeque<String>,
}
fn valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 1024
        && value
            .bytes()
            .all(|b| matches!(b,0x21|0x23..=0x2b|0x2d..=0x3a|0x3c..=0x5b|0x5d..=0x7e))
}
impl Tokens {
    pub fn seed(&mut self, value: &str) -> Result<(), Failure> {
        if !valid(value) {
            return Err(Failure::Invalid);
        }
        self.values = VecDeque::from([value.to_owned()]);
        Ok(())
    }
    pub fn replace(&mut self, headers: &http::HeaderMap) -> Result<bool, Failure> {
        let read = |key| -> Result<Option<String>, Failure> {
            let mut values = headers.get_all(key).iter();
            let first = values.next();
            if values.next().is_some() {
                return Err(Failure::Invalid);
            }
            first
                .map(|v| {
                    v.to_str()
                        .ok()
                        .filter(|s| valid(s))
                        .map(str::to_owned)
                        .ok_or(Failure::Invalid)
                })
                .transpose()
        };
        let one = read("__requestverificationtokenone")?;
        let two = read("__requestverificationtokentwo")?;
        let single = read("__requestverificationtoken")?;
        if single.is_some() && (one.is_some() || two.is_some()) || two.is_some() && one.is_none() {
            return Err(Failure::Invalid);
        }
        if let Some(one) = one {
            self.values = VecDeque::from([one]);
        } else if let Some(single) = single {
            self.values = VecDeque::from([single]);
            return Ok(true);
        } else {
            return Ok(false);
        }
        if let Some(two) = two {
            self.values.push_back(two);
        }
        Ok(true)
    }
    // The pinned wire contract includes a GET token only when there is one.
    pub fn for_read(&self) -> Option<String> {
        if self.values.len() == 1 {
            self.values.front().cloned()
        } else {
            None
        }
    }
    pub fn take(&mut self) -> Option<String> {
        if self.values.len() > 1 {
            self.values.pop_front()
        } else {
            self.values.front().cloned()
        }
    }
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
    pub fn clear(&mut self) {
        self.values.clear();
    }
}
