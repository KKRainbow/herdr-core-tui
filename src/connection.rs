use herdr_client_lib::{
    client::{Client, Error, Notifications, Stream},
    transport::HcConfig,
};
use serde_json::{Value, json};
use std::time::Duration;
const DEADLINE: Duration = Duration::from_secs(3);

pub struct Connection {
    pub client: Client,
    pub events: Notifications,
    pub stream: Option<Stream>,
    pub address: String,
    pub record: Value,
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
        let (result,stream)=client.attach(json!({"terminal":target,"cols":size.0,"rows":size.1,"resumed":resumed,"history":1000}),DEADLINE).await?;
        let record = result["terminal"].clone();
        let address = record["address"].as_str().unwrap_or(target).to_owned();
        Ok(Self {
            client,
            events: Notifications::new(events),
            stream: Some(stream),
            address,
            record,
        })
    }
    pub async fn attach(&mut self, size: (u16, u16)) -> Result<(), Error> {
        let (result,stream)=self.client.attach(json!({"terminal":self.address,"cols":size.0,"rows":size.1,"resumed":true,"history":1000}),DEADLINE).await?;
        self.record = result["terminal"].clone();
        self.stream = Some(stream);
        Ok(())
    }
    pub async fn notification(&mut self, value: Value) -> Result<bool, Error> {
        if value["method"] == "missed" {
            if let Some(record) = value["params"]["terminals"]
                .as_array()
                .and_then(|a| a.iter().find(|r| r["address"] == self.address))
            {
                self.record = record.clone();
            }
            return Ok(true);
        }
        let event = &value["params"];
        if event["terminal"] != self.address {
            return Ok(false);
        }
        let mut records =
            std::collections::BTreeMap::from([(self.address.clone(), self.record.take())]);
        herdr_client_lib::sync::reduce(&mut records, event);
        self.record = records.remove(&self.address).unwrap_or(Value::Null);
        if event["type"] == "restarted" {
            self.stream = None;
            return Ok(true);
        }
        Ok(event["type"] == "size")
    }
    pub fn size(&self) -> (u16, u16) {
        (
            self.record["cols"].as_u64().unwrap_or(80).clamp(1, 65535) as u16,
            self.record["rows"].as_u64().unwrap_or(24).clamp(1, 65535) as u16,
        )
    }
}
