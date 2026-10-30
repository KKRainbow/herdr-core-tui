use herdr_client_lib::{
    client::{Client, Error, Notifications, Stream},
    protocol::{AttachParams, Notification, Terminal},
    transport::HcConfig,
};
use serde_json::Value;
use std::time::Duration;
const DEADLINE: Duration = Duration::from_secs(3);

pub struct Connection {
    pub client: Client,
    pub events: Notifications,
    pub stream: Option<Stream>,
    pub address: String,
    pub record: Terminal,
}
/// Attach with recent history, as Herdr shows on reattach.
pub fn attach_params(terminal: &str, size: (u16, u16), resumed: bool) -> AttachParams {
    AttachParams {
        terminal: terminal.into(),
        cols: size.0,
        rows: size.1,
        resumed,
        history: 1000,
        ..Default::default()
    }
}
impl Connection {
    pub async fn connect(
        config: &HcConfig,
        target: &str,
        size: (u16, u16),
        resumed: bool,
    ) -> Result<Self, Error> {
        let client = config.connect(DEADLINE).await?;
        let (_, events) = client.subscribe(DEADLINE).await?;
        let (record, stream) = client
            .attach(&attach_params(target, size, resumed), DEADLINE)
            .await?;
        let address = Some(record.address.clone())
            .filter(|a| !a.is_empty())
            .unwrap_or_else(|| target.to_owned());
        Ok(Self {
            client,
            events: Notifications::new(events),
            stream: Some(stream),
            address,
            record,
        })
    }
    pub async fn attach(&mut self, size: (u16, u16)) -> Result<(), Error> {
        let (record, stream) = self
            .client
            .attach(&attach_params(&self.address, size, true), DEADLINE)
            .await?;
        self.record = record;
        self.stream = Some(stream);
        Ok(())
    }
    pub async fn notification(&mut self, value: Value) -> Result<bool, Error> {
        let event = match Notification::decode(value)? {
            Notification::Missed(snapshot) => {
                if let Some(record) = snapshot
                    .terminals
                    .into_iter()
                    .find(|r| r.address == self.address)
                {
                    self.record = record;
                }
                return Ok(true);
            }
            Notification::Event(event) => event,
            _ => return Ok(false),
        };
        if event.terminal.as_deref() != Some(self.address.as_str()) {
            return Ok(false);
        }
        let mut records = std::collections::BTreeMap::from([(
            self.address.clone(),
            std::mem::take(&mut self.record),
        )]);
        herdr_client_lib::sync::reduce(&mut records, &event);
        self.record = records.remove(&self.address).unwrap_or_default();
        if event.kind == "restarted" {
            self.stream = None;
            return Ok(true);
        }
        Ok(event.kind == "size")
    }
    pub fn size(&self) -> (u16, u16) {
        size(&self.record, (80, 24))
    }
}
/// The terminal's authoritative size, or the fallback before core reports one.
pub fn size(record: &Terminal, fallback: (u16, u16)) -> (u16, u16) {
    (
        Some(record.cols).filter(|n| *n > 0).unwrap_or(fallback.0),
        Some(record.rows).filter(|n| *n > 0).unwrap_or(fallback.1),
    )
}
