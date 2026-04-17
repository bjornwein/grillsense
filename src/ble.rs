#![allow(dead_code)]
/// BLE provisioning for the GrillSense thermometer.
///
/// Uses the btleplug crate to scan for and configure the device via BLE.
/// The device advertises as "Thermo-typ*" and accepts AT commands over
/// GATT characteristic fff3 (write), with responses on fff1 (notify).
///
/// NOTE: This module requires the `ble` feature and the btleplug crate.
/// It is structured as a standalone module that can be enabled later.
use crate::protocol::ble::*;

/// BLE provisioning configuration.
#[derive(Debug, Clone)]
pub struct ProvisionConfig {
    /// WiFi SSID to configure on the device.
    pub wifi_ssid: String,
    /// WiFi password.
    pub wifi_password: String,
    /// UDP server address for temperature data (default: cloud server).
    pub server_host: String,
    /// UDP server port (default: 17000).
    pub server_port: u16,
}

impl ProvisionConfig {
    /// Create a config that sends data to the default cloud server.
    pub fn cloud_default(wifi_ssid: String, wifi_password: String) -> Self {
        Self {
            wifi_ssid,
            wifi_password,
            server_host: crate::protocol::CLOUD_HOST.to_string(),
            server_port: crate::protocol::udp::CLOUD_PORT,
        }
    }

    /// Create a config that sends data to a local server.
    pub fn local(wifi_ssid: String, wifi_password: String, local_ip: String, port: u16) -> Self {
        Self {
            wifi_ssid,
            wifi_password,
            server_host: local_ip,
            server_port: port,
        }
    }

    /// Generate the AT+NETP command for this config.
    pub fn netp_command(&self) -> String {
        format!(
            "AT+NETP=UDP,CLIENT,{},{}",
            self.server_port, self.server_host
        )
    }
}

/// Steps in the BLE provisioning sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProvisionStep {
    EnterAtMode,
    ConfirmAtMode,
    GetMac,
    SetSsid,
    SetPassword,
    SetServer,
    SetStaMode,
    Reboot,
    Done,
}

impl ProvisionStep {
    /// Get the AT command to send for this step.
    pub fn command(&self, config: &ProvisionConfig) -> Option<String> {
        match self {
            Self::EnterAtMode => Some(CMD_ENTER_AT.to_string()),
            Self::ConfirmAtMode => Some(CMD_CONFIRM_AT.to_string()),
            Self::GetMac => Some(CMD_GET_MAC.to_string()),
            Self::SetSsid => Some(format!("{CMD_SET_SSID_PREFIX}{}", config.wifi_ssid)),
            Self::SetPassword => {
                let prefix = if config.wifi_password.is_empty() {
                    CMD_SET_KEY_PREFIX_OPEN
                } else {
                    CMD_SET_KEY_PREFIX_WPA2
                };
                Some(format!("{prefix}{}", config.wifi_password))
            }
            Self::SetServer => Some(config.netp_command()),
            Self::SetStaMode => Some(CMD_SET_STA.to_string()),
            Self::Reboot => Some(CMD_REBOOT.to_string()),
            Self::Done => None,
        }
    }

    /// Whether this step's command should have \r\n appended (for BLE framing).
    pub fn append_crlf(&self) -> bool {
        !matches!(self, Self::EnterAtMode | Self::ConfirmAtMode)
    }

    /// Advance to the next step.
    pub fn next(&self) -> Self {
        match self {
            Self::EnterAtMode => Self::ConfirmAtMode,
            Self::ConfirmAtMode => Self::GetMac,
            Self::GetMac => Self::SetSsid,
            Self::SetSsid => Self::SetPassword,
            Self::SetPassword => Self::SetServer,
            Self::SetServer => Self::SetStaMode,
            Self::SetStaMode => Self::Reboot,
            Self::Reboot => Self::Done,
            Self::Done => Self::Done,
        }
    }

    /// Check if a BLE notify response indicates success for this step.
    pub fn is_success_response(&self, response: &str) -> bool {
        match self {
            Self::EnterAtMode => response == "a" || response == "+ERR",
            _ => response.starts_with("+ok"),
        }
    }

    /// Extract the MAC address from a GetMac response ("+ok=AA:BB:CC:DD:EE:FF").
    pub fn parse_mac_response(response: &str) -> Option<String> {
        if response.starts_with("+ok=") {
            Some(response.strip_prefix("+ok=").unwrap().to_string())
        } else {
            None
        }
    }
}

/// Generate BLE write packets for a provisioning step.
pub fn packets_for_step(step: ProvisionStep, config: &ProvisionConfig) -> Vec<Vec<u8>> {
    if let Some(cmd) = step.command(config) {
        frame_command(&cmd, step.append_crlf())
    } else {
        vec![]
    }
}

/// Print a summary of the provisioning sequence for debugging.
pub fn print_provision_sequence(config: &ProvisionConfig) {
    println!("BLE Provisioning Sequence:");
    println!("==========================");
    let mut step = ProvisionStep::EnterAtMode;
    let mut n = 1;
    while step != ProvisionStep::Done {
        if let Some(cmd) = step.command(config) {
            let packets = packets_for_step(step, config);
            println!(
                "  Step {n}: {step:?} → \"{cmd}\" ({} BLE packet{})",
                packets.len(),
                if packets.len() == 1 { "" } else { "s" }
            );
        }
        step = step.next();
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provision_steps_advance() {
        let mut step = ProvisionStep::EnterAtMode;
        let mut count = 0;
        while step != ProvisionStep::Done {
            step = step.next();
            count += 1;
        }
        assert_eq!(count, 8);
    }

    #[test]
    fn test_netp_command() {
        let config = ProvisionConfig::local(
            "MyWiFi".into(),
            "pass123".into(),
            "192.168.1.100".into(),
            17000,
        );
        assert_eq!(
            config.netp_command(),
            "AT+NETP=UDP,CLIENT,17000,192.168.1.100"
        );
    }

    #[test]
    fn test_packets_for_step() {
        let config = ProvisionConfig::cloud_default("TestSSID".into(), "TestPassword".into());

        let packets = packets_for_step(ProvisionStep::EnterAtMode, &config);
        assert_eq!(packets.len(), 1);
        assert_eq!(&packets[0][2..], b"+++"); // no \r\n for step 1

        let packets = packets_for_step(ProvisionStep::GetMac, &config);
        assert_eq!(packets.len(), 1);
        assert_eq!(&packets[0][2..], b"AT+WSMAC\r\n"); // \r\n for step 3+
    }

    #[test]
    fn test_success_response() {
        assert!(ProvisionStep::EnterAtMode.is_success_response("a"));
        assert!(ProvisionStep::EnterAtMode.is_success_response("+ERR"));
        assert!(!ProvisionStep::EnterAtMode.is_success_response("+ok"));

        assert!(ProvisionStep::ConfirmAtMode.is_success_response("+ok"));
        assert!(ProvisionStep::GetMac.is_success_response("+ok=AA:BB:CC"));
        assert!(!ProvisionStep::GetMac.is_success_response("+ERR"));
    }

    #[test]
    fn test_parse_mac() {
        assert_eq!(
            ProvisionStep::parse_mac_response("+ok=AA:BB:CC:DD:EE:FF"),
            Some("AA:BB:CC:DD:EE:FF".into())
        );
        assert_eq!(ProvisionStep::parse_mac_response("+ERR"), None);
    }
}
