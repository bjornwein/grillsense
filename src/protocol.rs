#![allow(dead_code)]
//! Protocol constants and types for the GrillSense thermometer.

/// Cloud API base host.
pub const CLOUD_HOST: &str = "smartserver.emaxtime.cn";

/// Cloud API base URL.
pub const CLOUD_BASE_URL: &str = "https://smartserver.emaxtime.cn/V1.0/";

/// BLE GATT UUIDs.
pub mod ble {
    pub const SERVICE_UUID: &str = "0000fff0-0000-1000-8000-00805f9b34fb";
    pub const NOTIFY_UUID: &str = "0000fff1-0000-1000-8000-00805f9b34fb";
    pub const WRITE_UUID: &str = "0000fff3-0000-1000-8000-00805f9b34fb";

    /// BLE device name prefix used during scanning.
    pub const DEVICE_NAME_PREFIX: &str = "Thermo-typ";

    /// AT commands used during BLE provisioning.
    pub const CMD_ENTER_AT: &str = "+++";
    pub const CMD_CONFIRM_AT: &str = "a";
    pub const CMD_GET_MAC: &str = "AT+WSMAC";
    pub const CMD_SET_SSID_PREFIX: &str = "AT+WSSSID=";
    pub const CMD_SET_KEY_PREFIX: &str = "AT+WSKEY=OPEN,NONE,";
    pub const CMD_SET_SERVER: &str = "AT+NETP=UDP,CLIENT,17000,smartserver.emaxtime.cn";
    pub const CMD_SET_STA: &str = "AT+WMODE=STA";
    pub const CMD_REBOOT: &str = "AT+Z";

    /// Maximum BLE payload per chunk (20-byte MTU minus 2-byte header).
    pub const MAX_CHUNK_PAYLOAD: usize = 18;

    /// Frame a command string into BLE write chunks.
    ///
    /// Returns a Vec of byte vectors, each ≤20 bytes, suitable for GATT writes.
    /// `append_crlf` should be true for AT commands (steps 3+), false for steps 1-2.
    pub fn frame_command(cmd: &str, append_crlf: bool) -> Vec<Vec<u8>> {
        let data = if append_crlf {
            format!("{cmd}\r\n")
        } else {
            cmd.to_string()
        };
        let bytes = data.as_bytes();
        let total_chunks = bytes.len().div_ceil(MAX_CHUNK_PAYLOAD);
        let total_chunks = total_chunks.min(3) as u8;

        let mut chunks = Vec::new();
        for i in 0..total_chunks {
            let start = i as usize * MAX_CHUNK_PAYLOAD;
            let end = ((i as usize + 1) * MAX_CHUNK_PAYLOAD).min(bytes.len());
            let mut chunk = Vec::with_capacity(2 + end - start);
            chunk.push(b'1' + i); // sequence: '1', '2', '3'
            chunk.push(total_chunks);
            chunk.extend_from_slice(&bytes[start..end]);
            chunks.push(chunk);
        }
        chunks
    }
}

/// WiFi AP mode constants.
pub mod ap {
    pub const DEFAULT_SSID: &str = "LivingSmart";
    pub const DEFAULT_IP: &str = "10.10.100.254";
    pub const DEFAULT_PORT: u16 = 8800;

    pub const CMD_HANDSHAKE: &str = "HF-A11ASSISTHREAD";
    pub const CMD_ACK: &str = "+ok";
}

/// LAN discovery constants.
pub mod lan {
    /// UDP port for Hi-Flying module discovery and AT commands.
    pub const DISCOVERY_PORT: u16 = 48899;
    /// Magic handshake string for discovery.
    pub const DISCOVERY_MAGIC: &str = "HF-A11ASSISTHREAD";
    /// Response to enter AT command mode after discovery.
    pub const AT_MODE_ENTER: &str = "+ok";
}

/// UDP protocol constants.
pub mod udp {
    pub const CLOUD_PORT: u16 = 17000;
    pub const ALT_CLOUD_IP: &str = "47.52.149.125";
    pub const ALT_CLOUD_PORT: u16 = 10000;

    // Binary packet framing
    pub const START_BYTE: u8 = 0x3C; // '<'
    pub const END_BYTE: u8 = 0x3E; // '>'
    pub const TYPE_TEMP: u8 = 0x54; // 'T' — temperature packet

    /// Fixed packet length for temperature reports.
    pub const TEMP_PACKET_LEN: usize = 18;

    /// Direction byte values.
    pub const DIR_DEVICE_TO_CLOUD: u8 = 0x00;
    pub const DIR_CLOUD_TO_DEVICE: u8 = 0x01;

    /// Parsed binary temperature packet.
    ///
    /// 18-byte format:
    /// ```text
    /// Offset  Len  Field
    /// 0       1    Start delimiter '<' (0x3C)
    /// 1       1    Packet type 'T' (0x54)
    /// 2       5    Device ID bytes (e.g. 02 CC 44 55 66)
    /// 7       2    Unit/config bytes (ASCII "00" = Celsius)
    /// 9       1    Direction: 0x00=device→cloud, 0x01=cloud→device
    /// 10      1    Channel count / flag (0x04 = 4 temp bytes = 2 channels)
    /// 11      2    Temperature ch1 (u16 big-endian, value/10 = °C)
    /// 13      2    Temperature ch2 (u16 big-endian, value/10 = °C)
    /// 15      2    Padding byte (0x00) + Checksum
    /// 17      1    End delimiter '>' (0x3E)
    /// ```
    ///
    /// Checksum = (sum(bytes[1..16]) + 0x3C) & 0xFF
    #[derive(Debug, Clone, PartialEq)]
    pub struct TempPacket {
        pub device_id: String,
        pub direction: u8,
        pub temp_ch1: f64,
        pub temp_ch2: f64,
        pub raw: Vec<u8>,
    }

    impl TempPacket {
        /// Parse a raw 18-byte temperature packet.
        pub fn parse(data: &[u8]) -> Option<Self> {
            if data.len() != TEMP_PACKET_LEN {
                return None;
            }
            if data[0] != START_BYTE || data[17] != END_BYTE || data[1] != TYPE_TEMP {
                return None;
            }

            // Verify checksum
            let expected_checksum = compute_checksum(&data[1..16]);
            if data[16] != expected_checksum {
                return None;
            }

            let device_id = data[2..7]
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<String>();

            let temp_ch1 = u16::from_be_bytes([data[11], data[12]]) as f64 / 10.0;
            let temp_ch2 = u16::from_be_bytes([data[13], data[14]]) as f64 / 10.0;

            Some(TempPacket {
                device_id,
                direction: data[9],
                temp_ch1,
                temp_ch2,
                raw: data.to_vec(),
            })
        }

        /// Build a temperature packet (for constructing echo responses, etc.).
        pub fn build(
            device_id_bytes: &[u8; 5],
            direction: u8,
            temp_ch1: u16,
            temp_ch2: u16,
        ) -> Vec<u8> {
            let mut pkt = vec![START_BYTE, TYPE_TEMP];
            pkt.extend_from_slice(device_id_bytes);
            pkt.extend_from_slice(&[0x30, 0x30]); // "00" unit config
            pkt.push(direction);
            pkt.push(0x04); // 4 temp bytes
            pkt.extend_from_slice(&temp_ch1.to_be_bytes());
            pkt.extend_from_slice(&temp_ch2.to_be_bytes());
            pkt.push(0x00); // padding
            let checksum = compute_checksum(&pkt[1..]);
            pkt.push(checksum);
            pkt.push(END_BYTE);
            pkt
        }

        /// Active (non-zero) channels with 1-based index.
        pub fn active_channels(&self) -> Vec<(usize, f64)> {
            let mut ch = Vec::new();
            if self.temp_ch1 != 0.0 {
                ch.push((1, self.temp_ch1));
            }
            if self.temp_ch2 != 0.0 {
                ch.push((2, self.temp_ch2));
            }
            ch
        }

        /// Convert to a cloud-API-compatible TempResult.
        pub fn to_temp_result(&self) -> super::TempResult {
            super::TempResult {
                is_online: false,
                isonline: true,
                time: String::new(),
                temperature_ch1: self.temp_ch1,
                temperature_ch2: self.temp_ch2,
                temperature_ch3: 0.0,
                temperature_ch4: 0.0,
                temperature_ch5: 0.0,
                temperature_ch6: 0.0,
            }
        }
    }

    /// Compute the checksum for bytes between '<' and the checksum position.
    /// checksum = (sum(content_bytes) + 0x3C) & 0xFF
    pub fn compute_checksum(content: &[u8]) -> u8 {
        let sum: u32 = content.iter().map(|&b| b as u32).sum();
        ((sum + START_BYTE as u32) & 0xFF) as u8
    }

    // Alarm packet constants
    /// Config bytes for alarm channel 1: ASCII "A1"
    pub const CONFIG_ALARM_CH1: [u8; 2] = [0x41, 0x31]; // 'A', '1'
    /// Config bytes for alarm channel 2: ASCII "A2"
    pub const CONFIG_ALARM_CH2: [u8; 2] = [0x41, 0x32]; // 'A', '2'
    /// Fixed packet length for alarm commands.
    pub const ALARM_PACKET_LEN: usize = 16;

    /// Build an alarm packet to send to the device.
    ///
    /// 16-byte format (captured from cloud):
    /// ```text
    /// [0]     0x3C      Start
    /// [1]     0x54      Type 'T'
    /// [2-6]   devid     Device ID (5 bytes)
    /// [7-8]   "A1"/"A2" Alarm config (channel)
    /// [9]     0x00      Direction (cloud→device)
    /// [10]    0x02      Data byte count
    /// [11]    0x00      High byte / padding
    /// [12-13] u16 LE    Alarm temp (value × 10, little-endian)
    /// [14]    checksum  (sum(bytes[1..14]) + 0x3C) & 0xFF
    /// [15]    0x3E      End
    /// ```
    pub fn build_alarm_packet(
        device_id_bytes: &[u8; 5],
        channel: u8,
        temp_celsius: f64,
    ) -> Vec<u8> {
        let config = match channel {
            2 => CONFIG_ALARM_CH2,
            _ => CONFIG_ALARM_CH1,
        };
        let raw_temp = (temp_celsius * 10.0) as u16;

        let mut pkt = vec![START_BYTE, TYPE_TEMP];
        pkt.extend_from_slice(device_id_bytes);
        pkt.extend_from_slice(&config);
        pkt.push(0x00); // direction
        pkt.push(0x02); // data byte count
        pkt.push(0x00); // high byte / padding
        pkt.push((raw_temp & 0xFF) as u8); // low byte (LE)
        pkt.push(((raw_temp >> 8) & 0xFF) as u8); // high byte (LE)
        let checksum = compute_checksum(&pkt[1..]);
        pkt.push(checksum);
        pkt.push(END_BYTE);
        debug_assert_eq!(pkt.len(), ALARM_PACKET_LEN);
        pkt
    }

    /// Parse an alarm packet. Returns (channel, temp_celsius) if valid.
    pub fn parse_alarm_packet(data: &[u8]) -> Option<(u8, f64)> {
        if data.len() != ALARM_PACKET_LEN {
            return None;
        }
        if data[0] != START_BYTE || data[15] != END_BYTE || data[1] != TYPE_TEMP {
            return None;
        }
        // Check config bytes for alarm
        let channel = match (data[7], data[8]) {
            (0x41, 0x31) => 1, // 'A1'
            (0x41, 0x32) => 2, // 'A2'
            _ => return None,
        };
        // Verify checksum
        let expected = compute_checksum(&data[1..14]);
        if data[14] != expected {
            return None;
        }
        // Alarm temp: u16 little-endian at bytes 12-13, ÷10
        let raw_temp = (data[12] as u16) | ((data[13] as u16) << 8);
        let temp_celsius = raw_temp as f64 / 10.0;
        Some((channel, temp_celsius))
    }

    /// Parse the device ID bytes from a raw packet (works for both temp and alarm).
    pub fn parse_device_id_bytes(data: &[u8]) -> Option<[u8; 5]> {
        if data.len() < 7 {
            return None;
        }
        let mut id = [0u8; 5];
        id.copy_from_slice(&data[2..7]);
        Some(id)
    }
}

use serde::Deserialize;

/// Temperature reading from the cloud API.
#[derive(Debug, Clone, Deserialize)]
pub struct TempResult {
    #[serde(default)]
    pub is_online: bool,
    #[serde(rename = "isonline", default)]
    pub isonline: bool,
    #[serde(default)]
    pub time: String,
    #[serde(default)]
    pub temperature_ch1: f64,
    #[serde(default)]
    pub temperature_ch2: f64,
    #[serde(default)]
    pub temperature_ch3: f64,
    #[serde(default)]
    pub temperature_ch4: f64,
    #[serde(default)]
    pub temperature_ch5: f64,
    #[serde(default)]
    pub temperature_ch6: f64,
}

impl TempResult {
    /// Check if device is online (handles both field names).
    pub fn online(&self) -> bool {
        self.is_online || self.isonline
    }

    /// Get all channel temperatures as an array.
    pub fn channels(&self) -> [f64; 6] {
        [
            self.temperature_ch1,
            self.temperature_ch2,
            self.temperature_ch3,
            self.temperature_ch4,
            self.temperature_ch5,
            self.temperature_ch6,
        ]
    }

    /// Get channel temperatures that have probes connected (non-zero).
    pub fn active_channels(&self) -> Vec<(usize, f64)> {
        self.channels()
            .iter()
            .enumerate()
            .filter(|&(_, t)| *t != 0.0)
            .map(|(i, &t)| (i + 1, t))
            .collect()
    }
}

/// Derive the cloud device ID from a WiFi MAC address.
///
/// The app transforms the WiFi MAC by removing the first 2 bytes (4 hex chars)
/// and prepending "02". For example:
///   WiFi MAC: AABBCC445566 → Device ID: 02CC445566
///
/// This is confirmed to work with the cloud API for G002 (HF-LPT230) devices.
pub fn wifi_mac_to_device_id(wifi_mac: &str) -> String {
    let stripped = wifi_mac.replace([':', '-'], "").to_uppercase();
    if stripped.len() >= 12 {
        format!("02{}", &stripped[4..])
    } else {
        // Fallback: return as-is if MAC is too short
        stripped
    }
}

/// Device info from the cloud API.
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceInfo {
    pub id: i64,
    pub mac: String,
    #[serde(default)]
    pub city: String,
    #[serde(default)]
    pub ip: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub isonline: bool,
    #[serde(default)]
    pub serial: i64,
    #[serde(default)]
    pub timezone: String,
    #[serde(default)]
    pub r#type: i32,
}

/// User info returned after login.
#[derive(Debug, Clone, Deserialize)]
pub struct UserInfo {
    pub id: i64,
    #[serde(default)]
    pub nickname: String,
    #[serde(default)]
    pub email: String,
    pub token: String,
    #[serde(default)]
    pub sex: i32,
}

/// Error response from the cloud API.
#[derive(Debug, Clone, Deserialize)]
pub struct ApiError {
    #[serde(default)]
    pub result: Option<i32>,
    #[serde(default)]
    pub info: Option<String>,
    #[serde(default)]
    pub error_code: Option<String>,
    #[serde(default)]
    pub error_message: Option<String>,
}

impl ApiError {
    /// Check if this represents an actual error (has error_code or non-zero result).
    pub fn is_error(&self) -> bool {
        self.error_code.is_some() || matches!(self.result, Some(r) if r != 0)
    }

    /// Human-readable error description.
    pub fn description(&self) -> String {
        let mut parts = Vec::new();
        if let Some(code) = &self.error_code {
            parts.push(format!("error {code}"));
        }
        if let Some(msg) = &self.error_message {
            parts.push(msg.clone());
        }
        if let Some(info) = &self.info {
            parts.push(info.clone());
        }
        if let Some(result) = self.result
            && result != 0
            && self.error_code.is_none()
        {
            parts.push(format!("result code {result}"));
        }
        if parts.is_empty() {
            "unknown error".to_string()
        } else {
            parts.join(": ")
        }
    }
}

/// Known cloud API error codes.
pub mod error_codes {
    /// Device does not exist (设备不存在).
    pub const DEVICE_NOT_FOUND: &str = "102";
}

/// Temperature unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TempUnit {
    Celsius,
    Fahrenheit,
}

impl TempUnit {
    pub fn as_str(self) -> &'static str {
        match self {
            TempUnit::Celsius => "C",
            TempUnit::Fahrenheit => "F",
        }
    }
}

/// Convert Celsius to Fahrenheit.
pub fn celsius_to_fahrenheit(c: f64) -> f64 {
    (c * 9.0 / 5.0 + 32.0).round()
}

/// Convert Fahrenheit to Celsius.
pub fn fahrenheit_to_celsius(f: f64) -> f64 {
    ((f - 32.0) * 5.0 / 9.0).round()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ble_framing_short() {
        let chunks = ble::frame_command("+++", false);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0][0], b'1'); // sequence
        assert_eq!(chunks[0][1], 1); // total chunks
        assert_eq!(&chunks[0][2..], b"+++");
    }

    #[test]
    fn test_ble_framing_with_crlf() {
        let chunks = ble::frame_command("AT+WSMAC", true);
        assert_eq!(chunks.len(), 1);
        assert_eq!(&chunks[0][2..], b"AT+WSMAC\r\n");
    }

    #[test]
    fn test_ble_framing_multi_chunk() {
        // 20 chars + \r\n = 22 bytes → 2 chunks
        let cmd = "AT+WSSSID=MyNetworkXY";
        let chunks = ble::frame_command(cmd, true);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0][0], b'1');
        assert_eq!(chunks[0][1], 2);
        assert_eq!(chunks[0].len(), 20); // 2 header + 18 payload
        assert_eq!(chunks[1][0], b'2');
        assert_eq!(chunks[1][1], 2);
    }

    #[test]
    fn test_temp_conversion() {
        assert_eq!(celsius_to_fahrenheit(100.0), 212.0);
        assert_eq!(celsius_to_fahrenheit(0.0), 32.0);
        assert_eq!(fahrenheit_to_celsius(212.0), 100.0);
        assert_eq!(fahrenheit_to_celsius(32.0), 0.0);
    }

    #[test]
    fn test_wifi_mac_to_device_id() {
        // Real-world example: WiFi MAC AABBCC445566 → device ID 02CC445566
        assert_eq!(wifi_mac_to_device_id("AABBCC445566"), "02CC445566");
        // With colons
        assert_eq!(wifi_mac_to_device_id("AA:BB:CC:44:55:66"), "02CC445566");
        // Lowercase
        assert_eq!(wifi_mac_to_device_id("aabbcc445566"), "02CC445566");
        // With hyphens
        assert_eq!(wifi_mac_to_device_id("AA-BB-CC-44-55-66"), "02CC445566");
    }

    #[test]
    fn test_active_channels() {
        let temp = TempResult {
            is_online: false,
            isonline: true,
            time: String::new(),
            temperature_ch1: 21.6,
            temperature_ch2: 0.0,
            temperature_ch3: 0.0,
            temperature_ch4: 0.0,
            temperature_ch5: 0.0,
            temperature_ch6: 0.0,
        };
        assert!(temp.online());
        let active = temp.active_channels();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0], (1, 21.6));
    }

    #[test]
    fn test_udp_packet_parse() {
        // Real captured packet from device
        let data: Vec<u8> = vec![
            0x3C, 0x54, 0x02, 0xCC, 0x44, 0x55, 0x66, 0x30, 0x30, 0x00, 0x04, 0x00, 0xD8, 0x00,
            0x00, 0x00, 0x99, 0x3E,
        ];
        let pkt = udp::TempPacket::parse(&data).expect("should parse");
        assert_eq!(pkt.device_id, "02CC445566");
        assert_eq!(pkt.direction, udp::DIR_DEVICE_TO_CLOUD);
        assert!((pkt.temp_ch1 - 21.6).abs() < 0.01);
        assert_eq!(pkt.temp_ch2, 0.0);
    }

    #[test]
    fn test_udp_packet_parse_cloud_echo() {
        // Cloud echo has direction=1 and adjusted checksum
        let data: Vec<u8> = vec![
            0x3C, 0x54, 0x02, 0xCC, 0x44, 0x55, 0x66, 0x30, 0x30, 0x01, 0x04, 0x00, 0xD8, 0x00,
            0x00, 0x00, 0x9A, 0x3E,
        ];
        let pkt = udp::TempPacket::parse(&data).expect("should parse cloud echo");
        assert_eq!(pkt.direction, udp::DIR_CLOUD_TO_DEVICE);
        assert!((pkt.temp_ch1 - 21.6).abs() < 0.01);
    }

    #[test]
    fn test_udp_packet_build_roundtrip() {
        let dev_id: [u8; 5] = [0x02, 0xCC, 0x44, 0x55, 0x66];
        let built = udp::TempPacket::build(&dev_id, udp::DIR_DEVICE_TO_CLOUD, 216, 0);
        assert_eq!(built.len(), 18);
        let parsed = udp::TempPacket::parse(&built).expect("roundtrip should parse");
        assert_eq!(parsed.device_id, "02CC445566");
        assert!((parsed.temp_ch1 - 21.6).abs() < 0.01);
        assert_eq!(parsed.temp_ch2, 0.0);
    }

    #[test]
    fn test_udp_checksum() {
        // Verify checksum computation matches captured data
        let content: Vec<u8> = vec![
            0x54, 0x02, 0xCC, 0x44, 0x55, 0x66, 0x30, 0x30, 0x00, 0x04, 0x00, 0xD8, 0x00, 0x00,
            0x00,
        ];
        assert_eq!(udp::compute_checksum(&content), 0x99);
    }

    #[test]
    fn test_udp_to_temp_result() {
        let data: Vec<u8> = vec![
            0x3C, 0x54, 0x02, 0xCC, 0x44, 0x55, 0x66, 0x30, 0x30, 0x00, 0x04, 0x00, 0xD8, 0x00,
            0x00, 0x00, 0x99, 0x3E,
        ];
        let pkt = udp::TempPacket::parse(&data).unwrap();
        let result = pkt.to_temp_result();
        assert!(result.online());
        assert!((result.temperature_ch1 - 21.6).abs() < 0.01);
        assert_eq!(result.temperature_ch2, 0.0);
    }

    #[test]
    fn test_alarm_packet_parse_75c() {
        // Alarm packet: cloud set 75°C on channel 1 (anonymized device ID)
        let data: Vec<u8> = vec![
            0x3C, 0x54, 0x02, 0xCC, 0x44, 0x55, 0x66, 0x41, 0x31, 0x00, 0x02, 0x00, 0xEE, 0x02,
            0xC1, 0x3E,
        ];
        let (ch, temp) = udp::parse_alarm_packet(&data).expect("should parse");
        assert_eq!(ch, 1);
        assert!((temp - 75.0).abs() < 0.01);
    }

    #[test]
    fn test_alarm_packet_parse_100c() {
        // Alarm packet: cloud set 100°C on channel 1 (anonymized device ID)
        let data: Vec<u8> = vec![
            0x3C, 0x54, 0x02, 0xCC, 0x44, 0x55, 0x66, 0x41, 0x31, 0x00, 0x02, 0x00, 0xE8, 0x03,
            0xBC, 0x3E,
        ];
        let (ch, temp) = udp::parse_alarm_packet(&data).expect("should parse");
        assert_eq!(ch, 1);
        assert!((temp - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_alarm_packet_build_roundtrip() {
        let dev_id: [u8; 5] = [0x02, 0xCC, 0x44, 0x55, 0x66];
        let pkt = udp::build_alarm_packet(&dev_id, 1, 75.0);
        assert_eq!(pkt.len(), 16);
        let (ch, temp) = udp::parse_alarm_packet(&pkt).expect("roundtrip should parse");
        assert_eq!(ch, 1);
        assert!((temp - 75.0).abs() < 0.01);
    }

    #[test]
    fn test_alarm_packet_build_matches_captured() {
        // Verify our builder produces the exact same bytes as captured (anonymized)
        let dev_id: [u8; 5] = [0x02, 0xCC, 0x44, 0x55, 0x66];
        let pkt = udp::build_alarm_packet(&dev_id, 1, 75.0);
        let expected: Vec<u8> = vec![
            0x3C, 0x54, 0x02, 0xCC, 0x44, 0x55, 0x66, 0x41, 0x31, 0x00, 0x02, 0x00, 0xEE, 0x02,
            0xC1, 0x3E,
        ];
        assert_eq!(pkt, expected);
    }

    #[test]
    fn test_alarm_packet_ch2() {
        let dev_id: [u8; 5] = [0x02, 0xCC, 0x44, 0x55, 0x66];
        let pkt = udp::build_alarm_packet(&dev_id, 2, 80.0);
        assert_eq!(pkt[7], 0x41); // 'A'
        assert_eq!(pkt[8], 0x32); // '2'
        let (ch, temp) = udp::parse_alarm_packet(&pkt).expect("should parse ch2");
        assert_eq!(ch, 2);
        assert!((temp - 80.0).abs() < 0.01);
    }
}
