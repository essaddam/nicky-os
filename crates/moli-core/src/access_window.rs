//! Validated, half-open access windows. No clock or side effects.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccessWindow {
    start: u64,
    end: u64,
}

impl AccessWindow {
    #[must_use]
    pub fn new(start: u64, end: u64) -> Option<Self> {
        if start >= end {
            return None;
        }
        Some(Self { start, end })
    }

    #[must_use]
    pub fn start(self) -> u64 {
        self.start
    }

    #[must_use]
    pub fn end(self) -> u64 {
        self.end
    }

    #[must_use]
    pub fn allows(self, now: u64) -> bool {
        self.start <= now && now < self.end
    }
}
