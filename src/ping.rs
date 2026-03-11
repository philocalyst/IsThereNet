use std::net::IpAddr;
use std::time::Duration;
use surge_ping::{Client, Config, PingIdentifier, PingSequence, ICMP};

pub struct Pinger {
    client: Client,
    target: IpAddr,
    sequence: u16,
}

impl Pinger {
    pub fn new(target: IpAddr) -> Result<Self, surge_ping::SurgeError> {
        let config = Config::default();
        let client = if target.is_ipv4() {
            Client::new(&config)?
        } else {
            Client::new(&Config::builder().kind(ICMP::V6).build())?
        };

        Ok(Self {
            client,
            target,
            sequence: 0,
        })
    }

    /// Sends an ICMP echo request and returns the round-trip time in milliseconds,
    /// or None if the ping timed out.
    pub async fn ping(&mut self, timeout: Duration) -> Result<Option<f64>, PingError> {
        let mut pinger = self.client.pinger(self.target, PingIdentifier(rand_id())).await;
        pinger.timeout(timeout);

        let sequence = PingSequence(self.sequence);
        self.sequence = self.sequence.wrapping_add(1);

        let payload = [0u8; 12];
        match pinger.ping(sequence, &payload).await {
            Ok((_, rtt)) => Ok(Some(rtt.as_secs_f64() * 1000.0)),
            Err(surge_ping::SurgeError::Timeout { .. }) => Ok(None),
            Err(error) => Err(PingError::Surge(error)),
        }
    }
}

fn rand_id() -> u16 {
    (std::process::id() & 0xFFFF) as u16
}

#[derive(Debug)]
pub enum PingError {
    Surge(surge_ping::SurgeError),
}

impl std::fmt::Display for PingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PingError::Surge(error) => write!(f, "ping error: {error}"),
        }
    }
}

impl std::error::Error for PingError {}
