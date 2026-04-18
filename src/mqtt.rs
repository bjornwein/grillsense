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

    /// Alarm command topic for a channel (HA sends set commands here).
    pub fn alarm_command_topic(&self, channel: u8) -> String {
        format!(
            "grillsense/{}/alarm_ch{}/set",
            sanitize_id(&self.device_id),
            channel
        )
    }

    /// HA discovery topic for a number entity.
    fn number_discovery_topic(&self, object_id: &str) -> String {
        format!(
            "{}/number/grillsense_{}/{}/config",
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

        // Alarm setpoint number entities for channels 1 and 2
        for ch in 1..=2 {
            let alarm_config = json!({
                "name": format!("{} Alarm CH{ch}", self.device_name),
                "unique_id": format!("grillsense_{dev_id}_alarm_ch{ch}"),
                "command_topic": self.alarm_command_topic(ch),
                "state_topic": state_topic,
                "value_template": format!("{{{{ value_json.alarm_ch{ch} }}}}"),
                "unit_of_measurement": "°C",
                "min": 0,
                "max": 300,
                "step": 0.5,
                "mode": "box",
                "availability_topic": avail_topic,
                "device": device,
            });
            msgs.push((
                self.number_discovery_topic(&format!("alarm_ch{ch}")),
                serde_json::to_string(&alarm_config).unwrap(),
            ));
        }

        msgs
    }

    /// Generate a state payload from temperature data.
    pub fn state_payload(&self, temp: &crate::protocol::TempResult) -> String {
        serde_json::to_string(&json!({
            "temperature_ch1": temp.temperature_ch1,
            "temperature_ch2": temp.temperature_ch2,
            "temperature_ch3": temp.temperature_ch3,
            "temperature_ch4": temp.temperature_ch4,
            "temperature_ch5": temp.temperature_ch5,
            "temperature_ch6": temp.temperature_ch6,
            "is_online": temp.online(),
            "data_age_secs": temp.age_secs(),
        }))
        .unwrap()
    }
}

/// Run the MQTT-HA bridge with automatic reconnection.
///
/// Wraps `run_bridge` in a retry loop — if the MQTT connection drops,
/// waits 5 seconds and reconnects. Suitable for unmonitored services.
pub async fn run_bridge_with_reconnect(config: &MqttHaConfig, client: &CloudClient) -> Result<()> {
    let mut attempts = 0u32;
    loop {
        match run_bridge(config, client).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                attempts += 1;
                let delay = (5 * attempts).min(60);
                eprintln!("[mqtt] Bridge error: {e}");
                eprintln!("[mqtt] Reconnecting in {delay}s (attempt {attempts})...");
                tokio::time::sleep(Duration::from_secs(delay.into())).await;
            }
        }
    }
}

/// Run the MQTT-HA bridge, polling the cloud API and publishing to MQTT.
///
/// Subscribes to alarm command topics and forwards setpoints to the cloud API.
/// This function uses a simple TCP-based MQTT v3.1.1 implementation to avoid
/// pulling in a full MQTT crate. For production use, consider rumqttc.
pub async fn run_bridge(config: &MqttHaConfig, client: &CloudClient) -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    let addr = format!("{}:{}", config.broker_host, config.broker_port);
    let stream = TcpStream::connect(&addr)
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

    let (reader, mut writer) = stream.into_split();
    writer.write_all(&connect_packet).await?;

    // Read CONNACK
    let mut buf = [0u8; 4];
    let mut reader = reader;
    reader.read_exact(&mut buf).await?;
    if buf[0] != 0x20 || buf[3] != 0x00 {
        anyhow::bail!("MQTT CONNACK failed (return code: {})", buf[3]);
    }

    println!("Connected to MQTT broker at {addr}");

    // Publish discovery messages (retained)
    for (topic, payload) in config.discovery_messages() {
        let packet = build_mqtt_publish(&topic, payload.as_bytes(), true);
        writer.write_all(&packet).await?;
    }
    println!("Published HA discovery config for 9 entities (6 probes + online + 2 alarms)");

    // Subscribe to alarm command topics
    let alarm_ch1_topic = config.alarm_command_topic(1);
    let alarm_ch2_topic = config.alarm_command_topic(2);
    let sub_packet = build_mqtt_subscribe(&[alarm_ch1_topic.as_str(), alarm_ch2_topic.as_str()], 1);
    writer.write_all(&sub_packet).await?;
    println!("[mqtt] Subscribed to alarm commands: {alarm_ch1_topic}, {alarm_ch2_topic}");

    // Publish online availability
    let avail_packet = build_mqtt_publish(&config.availability_topic(), b"online", true);
    writer.write_all(&avail_packet).await?;

    // Track alarm setpoints for state publishing
    let mut alarm_ch1: f64 = 0.0;
    let mut alarm_ch2: f64 = 0.0;

    // Spawn reader task for incoming MQTT messages (alarm commands, PINGRESP)
    let (alarm_tx, mut alarm_rx) = tokio::sync::mpsc::channel::<(u8, f64)>(16);
    tokio::spawn(async move {
        let mut buf = [0u8; 1024];
        let mut partial = Vec::new();
        loop {
            match reader.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    partial.extend_from_slice(&buf[..n]);
                    while let Some(pkt_len) = mqtt_packet_len(&partial) {
                        if partial.len() < pkt_len {
                            break;
                        }
                        let pkt_data: Vec<u8> = partial.drain(..pkt_len).collect();
                        if let Some((topic, payload, _)) = parse_incoming_publish(&pkt_data)
                            && let Ok(text) = std::str::from_utf8(&payload)
                            && let Ok(temp) = text.trim().parse::<f64>()
                        {
                            let channel = if topic.contains("alarm_ch2") { 2 } else { 1 };
                            let _ = alarm_tx.try_send((channel, temp));
                        }
                    }
                }
            }
        }
    });

    println!(
        "Polling temperature every {:?}, publishing to MQTT...",
        config.poll_interval
    );

    // Main loop: poll temperature, handle alarm commands, keepalive
    let mut poll_interval = tokio::time::interval(config.poll_interval);
    poll_interval.tick().await; // consume immediate first tick

    loop {
        tokio::select! {
            _ = poll_interval.tick() => {
                match client.get_temperature().await {
                    Ok(temp) => {
                        let mut payload = serde_json::json!({
                            "temperature_ch1": temp.temperature_ch1,
                            "temperature_ch2": temp.temperature_ch2,
                            "temperature_ch3": temp.temperature_ch3,
                            "temperature_ch4": temp.temperature_ch4,
                            "temperature_ch5": temp.temperature_ch5,
                            "temperature_ch6": temp.temperature_ch6,
                            "is_online": temp.online(),
                            "data_age_secs": temp.age_secs(),
                        });
                        // Include alarm setpoints if set
                        if alarm_ch1 > 0.0 {
                            payload["alarm_ch1"] = serde_json::json!(alarm_ch1);
                        }
                        if alarm_ch2 > 0.0 {
                            payload["alarm_ch2"] = serde_json::json!(alarm_ch2);
                        }
                        let state = serde_json::to_string(&payload).unwrap();
                        let packet = build_mqtt_publish(&config.state_topic(), state.as_bytes(), false);
                        writer.write_all(&packet).await?;

                        let is_fresh = temp.online() && !temp.is_stale(60);
                        let status = if is_fresh { "online" } else { "offline" };
                        let avail =
                            build_mqtt_publish(&config.availability_topic(), status.as_bytes(), true);
                        writer.write_all(&avail).await?;
                    }
                    Err(e) => {
                        eprintln!("Temperature poll error: {e}");
                    }
                }

                // Send PINGREQ to keep connection alive
                writer.write_all(&[0xC0, 0x00]).await?;
            }
            cmd = alarm_rx.recv() => {
                let Some((channel, temp_c)) = cmd else { break };
                eprintln!("[mqtt] Alarm command: CH{channel} = {temp_c:.1}°C");
                match client.set_alarm_temp(channel, temp_c).await {
                    Ok(()) => {
                        match channel {
                            1 => alarm_ch1 = temp_c,
                            2 => alarm_ch2 = temp_c,
                            _ => {}
                        }
                        println!("[mqtt] Alarm CH{channel} set to {temp_c:.1}°C via cloud API");
                    }
                    Err(e) => {
                        eprintln!("[mqtt] Failed to set alarm via cloud: {e}");
                    }
                }
            }
        }
    }

    Ok(())
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

/// Build a minimal MQTT SUBSCRIBE packet.
///
/// Subscribes to a list of topic filters at QoS 0.
pub fn build_mqtt_subscribe(topics: &[&str], packet_id: u16) -> Vec<u8> {
    let mut payload = Vec::new();
    for topic in topics {
        mqtt_write_string(&mut payload, topic);
        payload.push(0x00); // QoS 0
    }

    let mut packet = Vec::new();
    packet.push(0x82); // SUBSCRIBE (0x80 | 0x02 for required reserved bits)
    let remaining = 2 + payload.len(); // 2 bytes for packet ID
    mqtt_encode_remaining_length(&mut packet, remaining);
    packet.push((packet_id >> 8) as u8);
    packet.push((packet_id & 0xFF) as u8);
    packet.extend(payload);
    packet
}

/// Decode the remaining length from an MQTT packet stream.
///
/// Returns (length, bytes_consumed) or None if incomplete.
fn mqtt_decode_remaining_length(data: &[u8]) -> Option<(usize, usize)> {
    let mut multiplier = 1usize;
    let mut value = 0usize;
    for (i, &byte) in data.iter().enumerate() {
        value += (byte & 0x7F) as usize * multiplier;
        if byte & 0x80 == 0 {
            return Some((value, i + 1));
        }
        multiplier *= 128;
        if multiplier > 128 * 128 * 128 {
            return None; // malformed
        }
    }
    None // incomplete
}

/// Parse an incoming MQTT PUBLISH packet from a byte buffer.
///
/// Returns `Some((topic, payload, total_bytes_consumed))` for PUBLISH packets,
/// or `None` for other packet types (PINGRESP, SUBACK, etc.) which are skipped.
pub fn parse_incoming_publish(data: &[u8]) -> Option<(String, Vec<u8>, usize)> {
    if data.is_empty() {
        return None;
    }
    let packet_type = data[0] & 0xF0;
    if packet_type != 0x30 {
        // Not a PUBLISH — skip the packet
        return None;
    }

    let (remaining_len, rl_bytes) = mqtt_decode_remaining_length(&data[1..])?;
    let header_len = 1 + rl_bytes;
    if data.len() < header_len + remaining_len {
        return None; // incomplete packet
    }

    let body = &data[header_len..header_len + remaining_len];
    if body.len() < 2 {
        return None;
    }
    let topic_len = ((body[0] as usize) << 8) | body[1] as usize;
    if body.len() < 2 + topic_len {
        return None;
    }
    let topic = String::from_utf8_lossy(&body[2..2 + topic_len]).to_string();
    let payload = body[2 + topic_len..].to_vec();
    let total = header_len + remaining_len;
    Some((topic, payload, total))
}

/// Calculate total packet length from an MQTT packet header.
///
/// Returns `Some(total_bytes)` or `None` if the remaining length is incomplete.
pub fn mqtt_packet_len(data: &[u8]) -> Option<usize> {
    if data.is_empty() {
        return None;
    }
    let (remaining_len, rl_bytes) = mqtt_decode_remaining_length(&data[1..])?;
    Some(1 + rl_bytes + remaining_len)
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
        assert_eq!(msgs.len(), 9); // ch1-6 + online + alarm_ch1 + alarm_ch2

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

        // Check alarm number entities
        let (topic, payload) = &msgs[7];
        assert!(topic.contains("number/"));
        assert!(topic.contains("alarm_ch1"));
        let v: serde_json::Value = serde_json::from_str(payload).unwrap();
        assert!(
            v["command_topic"]
                .as_str()
                .unwrap()
                .contains("alarm_ch1/set")
        );
        assert_eq!(v["min"], 0);
        assert_eq!(v["max"], 300);
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
            ..Default::default()
        };
        let payload = config.state_payload(&temp);
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(v["temperature_ch1"], 72.5);
        assert_eq!(v["temperature_ch2"], 0.0);
        assert_eq!(v["is_online"], true);
        assert!(v["data_age_secs"].is_null()); // no timestamp → null age
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

    #[test]
    fn test_mqtt_subscribe_packet() {
        let packet = build_mqtt_subscribe(&["test/topic"], 1);
        assert_eq!(packet[0], 0x82); // SUBSCRIBE type
        assert_eq!(packet[2], 0x00); // packet ID MSB
        assert_eq!(packet[3], 0x01); // packet ID LSB
        // topic "test/topic" length
        assert_eq!(packet[4], 0x00);
        assert_eq!(packet[5], 10);
        // QoS 0 at the end
        assert_eq!(packet[packet.len() - 1], 0x00);
    }

    #[test]
    fn test_parse_incoming_publish() {
        // Build a PUBLISH packet and parse it back
        let packet = build_mqtt_publish("alarm/set", b"75.5", false);
        let result = parse_incoming_publish(&packet);
        assert!(result.is_some());
        let (topic, payload, len) = result.unwrap();
        assert_eq!(topic, "alarm/set");
        assert_eq!(payload, b"75.5");
        assert_eq!(len, packet.len());
    }

    #[test]
    fn test_parse_incoming_non_publish() {
        // PINGRESP packet (0xD0)
        let pingresp = vec![0xD0, 0x00];
        assert!(parse_incoming_publish(&pingresp).is_none());
    }

    #[test]
    fn test_mqtt_packet_len() {
        let packet = build_mqtt_publish("t", b"hello", false);
        assert_eq!(mqtt_packet_len(&packet), Some(packet.len()));

        // Empty buffer
        assert_eq!(mqtt_packet_len(&[]), None);
    }
}
