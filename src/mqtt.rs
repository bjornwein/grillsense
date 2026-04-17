#![allow(dead_code)]
/// MQTT integration for Home Assistant auto-discovery.
///
/// Publishes temperature data as HA sensor entities via MQTT discovery protocol.
/// See: https://www.home-assistant.io/integrations/mqtt/#mqtt-discovery
use anyhow::{Context, Result};
use serde_json::json;
use std::time::Duration;

use crate::cloud::CloudClient;

/// MQTT topic prefix for Home Assistant discovery.
const HA_DISCOVERY_PREFIX: &str = "homeassistant";

/// Configuration for the MQTT Home Assistant bridge.
#[derive(Debug, Clone)]
pub struct MqttHaConfig {
    /// MQTT broker host.
    pub broker_host: String,
    /// MQTT broker port.
    pub broker_port: u16,
    /// MQTT username (optional).
    pub username: Option<String>,
    /// MQTT password (optional).
    pub password: Option<String>,
    /// Device name in Home Assistant.
    pub device_name: String,
    /// Unique device identifier (usually the MAC address).
    pub device_id: String,
    /// Polling interval for temperature data.
    pub poll_interval: Duration,
}

impl MqttHaConfig {
    /// State topic for temperature data.
    pub fn state_topic(&self) -> String {
        format!("grillsense/{}/state", sanitize_id(&self.device_id))
    }

    /// Availability topic.
    pub fn availability_topic(&self) -> String {
        format!("grillsense/{}/availability", sanitize_id(&self.device_id))
    }

    /// HA discovery topic for a sensor entity.
    fn discovery_topic(&self, object_id: &str) -> String {
        format!(
            "{}/sensor/grillsense_{}/{}/config",
            HA_DISCOVERY_PREFIX,
            sanitize_id(&self.device_id),
            object_id
        )
    }

    /// HA discovery topic for a binary sensor.
    fn binary_discovery_topic(&self, object_id: &str) -> String {
        format!(
            "{}/binary_sensor/grillsense_{}/{}/config",
            HA_DISCOVERY_PREFIX,
            sanitize_id(&self.device_id),
            object_id
        )
    }

    /// Generate the HA device info block for discovery payloads.
    fn device_info(&self) -> serde_json::Value {
        json!({
            "identifiers": [format!("grillsense_{}", sanitize_id(&self.device_id))],
            "name": self.device_name,
            "manufacturer": "Ezon/Dangrill",
            "model": "GrillSense WiFi BBQ Thermometer",
            "sw_version": "grillsense-rs"
        })
    }

    /// Generate all MQTT discovery payloads.
    ///
    /// Returns a Vec of (topic, payload_json) pairs to publish.
    pub fn discovery_messages(&self) -> Vec<(String, String)> {
        let state_topic = self.state_topic();
        let avail_topic = self.availability_topic();
        let device = self.device_info();
        let dev_id = sanitize_id(&self.device_id);

        let mut msgs = Vec::new();

        // Temperature sensors for all 6 channels
        for ch in 1..=6 {
            let config = json!({
                "name": format!("{} Probe {ch}", self.device_name),
                "unique_id": format!("grillsense_{dev_id}_ch{ch}"),
                "state_topic": state_topic,
                "value_template": format!("{{{{ value_json.temperature_ch{ch} }}}}"),
                "unit_of_measurement": "°C",
                "device_class": "temperature",
                "state_class": "measurement",
                "availability_topic": avail_topic,
                "device": device,
            });
            msgs.push((
                self.discovery_topic(&format!("ch{ch}")),
                serde_json::to_string(&config).unwrap(),
            ));
        }

        // Online status binary sensor
        let online_config = json!({
            "name": format!("{} Online", self.device_name),
            "unique_id": format!("grillsense_{dev_id}_online"),
            "state_topic": state_topic,
            "value_template": "{{ 'ON' if value_json.is_online else 'OFF' }}",
            "device_class": "connectivity",
            "availability_topic": avail_topic,
            "device": device,
        });
        msgs.push((
            self.binary_discovery_topic("online"),
            serde_json::to_string(&online_config).unwrap(),
        ));

        msgs
    }

    /// Generate a state payload from temperature data.
    pub fn state_payload(
        &self,
        temp: &crate::protocol::TempResult,
    ) -> String {
        serde_json::to_string(&json!({
            "temperature_ch1": temp.temperature_ch1,
            "temperature_ch2": temp.temperature_ch2,
            "temperature_ch3": temp.temperature_ch3,
            "temperature_ch4": temp.temperature_ch4,
            "temperature_ch5": temp.temperature_ch5,
            "temperature_ch6": temp.temperature_ch6,
            "is_online": temp.online(),
        }))
        .unwrap()
    }
}

/// Run the MQTT-HA bridge, polling the cloud API and publishing to MQTT.
///
/// This function uses a simple TCP-based MQTT v3.1.1 implementation to avoid
/// pulling in a full MQTT crate. For production use, consider rumqttc.
pub async fn run_bridge(config: &MqttHaConfig, client: &CloudClient) -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    let addr = format!("{}:{}", config.broker_host, config.broker_port);
    let mut stream = TcpStream::connect(&addr)
        .await
        .with_context(|| format!("Failed to connect to MQTT broker at {addr}"))?;

    // MQTT CONNECT packet
    let connect_packet = build_mqtt_connect(
        &format!("grillsense_{}", sanitize_id(&config.device_id)),
        config.username.as_deref(),
        config.password.as_deref(),
        // LWT: mark as offline on disconnect
        Some((&config.availability_topic(), "offline")),
    );
    stream.write_all(&connect_packet).await?;

    // Read CONNACK
    let mut buf = [0u8; 4];
    stream.read_exact(&mut buf).await?;
    if buf[0] != 0x20 || buf[3] != 0x00 {
        anyhow::bail!("MQTT CONNACK failed (return code: {})", buf[3]);
    }

    println!("Connected to MQTT broker at {addr}");

    // Publish discovery messages (retained)
    for (topic, payload) in config.discovery_messages() {
        let packet = build_mqtt_publish(&topic, payload.as_bytes(), true);
        stream.write_all(&packet).await?;
    }
    println!("Published HA discovery config for 7 entities (6 probes + online)");

    // Publish online availability
    let avail_packet =
        build_mqtt_publish(&config.availability_topic(), b"online", true);
    stream.write_all(&avail_packet).await?;

    println!(
        "Polling temperature every {:?}, publishing to MQTT...",
        config.poll_interval
    );

    // Main loop: poll temperature and publish
    loop {
        match client.get_temperature().await {
            Ok(temp) => {
                let payload = config.state_payload(&temp);
                let packet =
                    build_mqtt_publish(&config.state_topic(), payload.as_bytes(), false);
                stream.write_all(&packet).await?;

                // Also update availability
                let status = if temp.online() { "online" } else { "offline" };
                let avail = build_mqtt_publish(
                    &config.availability_topic(),
                    status.as_bytes(),
                    true,
                );
                stream.write_all(&avail).await?;
            }
            Err(e) => {
                eprintln!("Temperature poll error: {e}");
            }
        }

        // Send PINGREQ to keep connection alive
        stream.write_all(&[0xC0, 0x00]).await?;

        // Read any pending data (PINGRESP, etc.) — non-blocking
        let mut resp_buf = [0u8; 256];
        let _ = tokio::time::timeout(
            Duration::from_millis(100),
            stream.read(&mut resp_buf),
        )
        .await;

        tokio::time::sleep(config.poll_interval).await;
    }
}

/// Build a minimal MQTT v3.1.1 CONNECT packet.
pub fn build_mqtt_connect(
    client_id: &str,
    username: Option<&str>,
    password: Option<&str>,
    will: Option<(&str, &str)>,
) -> Vec<u8> {
    let mut variable = Vec::new();

    // Protocol Name
    variable.extend_from_slice(&[0x00, 0x04]); // length
    variable.extend_from_slice(b"MQTT");
    // Protocol Level (4 = v3.1.1)
    variable.push(0x04);
    // Connect Flags
    let mut flags: u8 = 0x02; // Clean Session
    if username.is_some() {
        flags |= 0x80;
    }
    if password.is_some() {
        flags |= 0x40;
    }
    if will.is_some() {
        flags |= 0x24; // Will Flag + Will Retain
    }
    variable.push(flags);
    // Keep Alive (60 seconds)
    variable.extend_from_slice(&[0x00, 0x3C]);

    // Payload
    let mut payload = Vec::new();
    mqtt_write_string(&mut payload, client_id);
    if let Some((topic, msg)) = will {
        mqtt_write_string(&mut payload, topic);
        mqtt_write_string(&mut payload, msg);
    }
    if let Some(u) = username {
        mqtt_write_string(&mut payload, u);
    }
    if let Some(p) = password {
        mqtt_write_string(&mut payload, p);
    }

    let mut packet = Vec::new();
    packet.push(0x10); // CONNECT packet type
    let remaining = variable.len() + payload.len();
    mqtt_encode_remaining_length(&mut packet, remaining);
    packet.extend(variable);
    packet.extend(payload);
    packet
}

/// Build a minimal MQTT PUBLISH packet.
pub fn build_mqtt_publish(topic: &str, payload: &[u8], retain: bool) -> Vec<u8> {
    let mut packet = Vec::new();
    let first_byte = 0x30 | if retain { 0x01 } else { 0x00 };
    packet.push(first_byte);

    let topic_bytes = topic.as_bytes();
    let remaining = 2 + topic_bytes.len() + payload.len();
    mqtt_encode_remaining_length(&mut packet, remaining);

    // Topic
    packet.push((topic_bytes.len() >> 8) as u8);
    packet.push((topic_bytes.len() & 0xFF) as u8);
    packet.extend_from_slice(topic_bytes);

    // Payload
    packet.extend_from_slice(payload);
    packet
}

fn mqtt_write_string(buf: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    buf.push((bytes.len() >> 8) as u8);
    buf.push((bytes.len() & 0xFF) as u8);
    buf.extend_from_slice(bytes);
}

fn mqtt_encode_remaining_length(buf: &mut Vec<u8>, mut len: usize) {
    loop {
        let mut byte = (len % 128) as u8;
        len /= 128;
        if len > 0 {
            byte |= 0x80;
        }
        buf.push(byte);
        if len == 0 {
            break;
        }
    }
}

/// Sanitize a string for use in MQTT topics and HA unique IDs.
fn sanitize_id(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_id() {
        assert_eq!(sanitize_id("AA:BB:CC:DD:EE:FF"), "AA_BB_CC_DD_EE_FF");
        assert_eq!(sanitize_id("simple"), "simple");
    }

    #[test]
    fn test_discovery_messages() {
        let config = MqttHaConfig {
            broker_host: "localhost".into(),
            broker_port: 1883,
            username: None,
            password: None,
            device_name: "BBQ".into(),
            device_id: "AA:BB:CC".into(),
            poll_interval: Duration::from_secs(3),
        };

        let msgs = config.discovery_messages();
        assert_eq!(msgs.len(), 7); // ch1-6 + online

        // Check ch1 discovery
        let (topic, payload) = &msgs[0];
        assert!(topic.contains("homeassistant/sensor/"));
        assert!(topic.ends_with("/config"));
        let v: serde_json::Value = serde_json::from_str(payload).unwrap();
        assert_eq!(v["unit_of_measurement"], "°C");
        assert_eq!(v["device_class"], "temperature");

        // Check online binary sensor
        let (topic, _) = &msgs[6];
        assert!(topic.contains("binary_sensor"));
    }

    #[test]
    fn test_state_payload() {
        let config = MqttHaConfig {
            broker_host: "localhost".into(),
            broker_port: 1883,
            username: None,
            password: None,
            device_name: "BBQ".into(),
            device_id: "test".into(),
            poll_interval: Duration::from_secs(3),
        };

        let temp = crate::protocol::TempResult {
            is_online: false,
            isonline: true,
            time: String::new(),
            temperature_ch1: 72.5,
            temperature_ch2: 0.0,
            temperature_ch3: 0.0,
            temperature_ch4: 0.0,
            temperature_ch5: 0.0,
            temperature_ch6: 0.0,
        };
        let payload = config.state_payload(&temp);
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(v["temperature_ch1"], 72.5);
        assert_eq!(v["temperature_ch2"], 0.0);
        assert_eq!(v["is_online"], true);
    }

    #[test]
    fn test_mqtt_connect_packet() {
        let packet = build_mqtt_connect("test_client", None, None, None);
        assert_eq!(packet[0], 0x10); // CONNECT type
        // Protocol name "MQTT" at offset after remaining length
        let rl_len = 1; // remaining length fits in 1 byte for small packets
        assert_eq!(&packet[1 + rl_len..1 + rl_len + 6], b"\x00\x04MQTT");
    }

    #[test]
    fn test_mqtt_publish_packet() {
        let packet = build_mqtt_publish("test/topic", b"hello", false);
        assert_eq!(packet[0], 0x30); // PUBLISH, no retain
        // Topic "test/topic" is 10 bytes
        assert_eq!(packet[2], 0x00); // topic length MSB
        assert_eq!(packet[3], 10); // topic length LSB

        let retain_packet = build_mqtt_publish("t", b"x", true);
        assert_eq!(retain_packet[0], 0x31); // PUBLISH with retain
    }

    #[test]
    fn test_mqtt_remaining_length() {
        let mut buf = Vec::new();
        mqtt_encode_remaining_length(&mut buf, 0);
        assert_eq!(buf, vec![0x00]);

        buf.clear();
        mqtt_encode_remaining_length(&mut buf, 127);
        assert_eq!(buf, vec![0x7F]);

        buf.clear();
        mqtt_encode_remaining_length(&mut buf, 128);
        assert_eq!(buf, vec![0x80, 0x01]);
    }
}
