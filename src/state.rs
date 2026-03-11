use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum NetStatus {
    Connected(f64),
    Slow(f64),
    Disconnected,
}

impl NetStatus {
    pub fn kind(&self) -> NetStatusKind {
        match self {
            NetStatus::Connected(_) => NetStatusKind::Connected,
            NetStatus::Slow(_) => NetStatusKind::Slow,
            NetStatus::Disconnected => NetStatusKind::Disconnected,
        }
    }

    pub fn rtt_ms(&self) -> f64 {
        match self {
            NetStatus::Connected(ms) => *ms,
            NetStatus::Slow(ms) => *ms,
            NetStatus::Disconnected => 0.0,
        }
    }
}

impl fmt::Display for NetStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetStatus::Connected(ms) => write!(f, "OK ({ms:.1} ms)"),
            NetStatus::Slow(ms) => write!(f, "SLOW ({ms:.1} ms)"),
            NetStatus::Disconnected => write!(f, "TIMEOUT"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetStatusKind {
    Connected,
    Slow,
    Disconnected,
}

impl fmt::Display for NetStatusKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetStatusKind::Connected => write!(f, "CONNECTED"),
            NetStatusKind::Slow => write!(f, "SLOW"),
            NetStatusKind::Disconnected => write!(f, "DISCONNECTED"),
        }
    }
}

const MAX_COUNTS: u32 = 2;

/// Debounces transitions between network states, mirroring the original Swift logic.
/// Requires multiple consecutive observations before accepting a state change.
pub struct Debouncer {
    slow_counter: u32,
    timeout_counter: u32,
    fast_counter: u32,
}

impl Default for Debouncer {
    fn default() -> Self {
        Self {
            slow_counter: MAX_COUNTS,
            timeout_counter: MAX_COUNTS,
            fast_counter: MAX_COUNTS,
        }
    }
}

impl Debouncer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Process a ping observation and return the new status if a transition should occur.
    /// `rtt_ms` is None for timeouts, Some(ms) for successful pings.
    /// `slow_threshold_ms` is the configured threshold for "slow" classification.
    /// `current` is the current status kind, or None if this is the first observation.
    pub fn observe(
        &mut self,
        rtt_ms: Option<f64>,
        slow_threshold_ms: f64,
        current: Option<NetStatusKind>,
    ) -> Option<NetStatus> {
        match rtt_ms {
            None => {
                self.fast_counter = MAX_COUNTS;
                self.slow_counter = MAX_COUNTS;

                if self.timeout_counter == 0 {
                    self.timeout_counter = MAX_COUNTS;
                    Some(NetStatus::Disconnected)
                } else {
                    self.timeout_counter -= 1;
                    None
                }
            }
            Some(ms) => {
                if current.is_none() || current == Some(NetStatusKind::Disconnected) {
                    self.slow_counter = MAX_COUNTS;
                    self.fast_counter = MAX_COUNTS;
                    self.timeout_counter = MAX_COUNTS;
                    if slow_threshold_ms > 0.0 && ms > slow_threshold_ms {
                        return Some(NetStatus::Slow(ms));
                    }
                    return Some(NetStatus::Connected(ms));
                }

                if ms > 160.0 {
                    self.fast_counter = MAX_COUNTS;
                    self.timeout_counter = MAX_COUNTS;

                    if self.slow_counter == 0 {
                        self.slow_counter = MAX_COUNTS;
                        Some(NetStatus::Slow(ms))
                    } else {
                        self.slow_counter -= 1;
                        None
                    }
                } else if current == Some(NetStatusKind::Slow) {
                    if ms >= 80.0 {
                        return None;
                    }
                    self.slow_counter = MAX_COUNTS;
                    self.timeout_counter = MAX_COUNTS;

                    if self.fast_counter == 0 {
                        self.fast_counter = MAX_COUNTS;
                        Some(NetStatus::Connected(ms))
                    } else {
                        self.fast_counter -= 1;
                        None
                    }
                } else {
                    self.slow_counter = MAX_COUNTS;
                    self.fast_counter = MAX_COUNTS;
                    self.timeout_counter = MAX_COUNTS;
                    Some(NetStatus::Connected(ms))
                }
            }
        }
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}
