#![allow(dead_code)]
/// Protocol constants and types for the GrillSense thermometer.

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
        let total_chunks = (bytes.len() + MAX_CHUNK_PAYLOAD - 1) / MAX_CHUNK_PAYLOAD;
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

/// UDP protocol constants.
pub mod udp {
    pub const CLOUD_PORT: u16 = 17000;
    pub const ALT_CLOUD_IP: &str = "47.52.149.125";
    pub const ALT_CLOUD_PORT: u16 = 10000;
}

use serde::Deserialize;

/// Temperature reading from the cloud API.
#[derive(Debug, Clone, Deserialize)]
pub struct TempResult {
    pub is_online: bool,
    pub temperature_ch1: f64,
    pub temperature_ch2: f64,
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
        if let Some(result) = self.result {
            if result != 0 && self.error_code.is_none() {
                parts.push(format!("result code {result}"));
            }
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
}
