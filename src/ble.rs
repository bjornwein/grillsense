#![allow(dead_code)]
/// BLE provisioning for the GrillSense thermometer.
///
/// The device advertises as "Thermo-typ*" and accepts AT commands over
/// GATT characteristic fff3 (write), with responses on fff1 (notify).
///
/// This module contains data structures that work without btleplug,
/// plus the actual BLE runtime (scan/connect/provision) behind the `ble` feature.
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
        response.strip_prefix("+ok=").map(|s| s.to_string())
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

// ── BLE runtime (requires btleplug) ──────────────────────────────────────

#[cfg(feature = "ble")]
pub mod runtime {
    use super::*;
    use anyhow::{Context, Result};
    use btleplug::api::{
        Central, Characteristic, Manager as _, Peripheral as _, ScanFilter, WriteType,
    };
    use btleplug::platform::{Adapter, Manager, Peripheral};
    use futures::StreamExt;
    use std::time::Duration;

    const SCAN_TIMEOUT: Duration = Duration::from_secs(10);
    const STEP_TIMEOUT: Duration = Duration::from_secs(5);
    const STEP_RETRIES: usize = 3;
    const INTER_STEP_DELAY: Duration = Duration::from_millis(500);

    const SERVICE: uuid::Uuid = uuid::Uuid::from_u128(0x0000fff0_0000_1000_8000_00805f9b34fb);
    const NOTIFY_CHAR: uuid::Uuid = uuid::Uuid::from_u128(0x0000fff1_0000_1000_8000_00805f9b34fb);
    const WRITE_CHAR: uuid::Uuid = uuid::Uuid::from_u128(0x0000fff3_0000_1000_8000_00805f9b34fb);

    async fn get_adapter() -> Result<Adapter> {
        let manager = Manager::new()
            .await
            .context("Failed to create BLE manager")?;
        let adapters = manager.adapters().await.context("No BLE adapters found")?;
        adapters
            .into_iter()
            .next()
            .context("No BLE adapter available")
    }

    fn find_characteristic(
        chars: &std::collections::BTreeSet<Characteristic>,
        uuid: uuid::Uuid,
    ) -> Result<Characteristic> {
        chars
            .iter()
            .find(|c| c.uuid == uuid)
            .cloned()
            .with_context(|| format!("Characteristic {uuid} not found"))
    }

    /// Scan for GrillSense devices. Returns list of (name, address, peripheral).
    pub async fn scan() -> Result<Vec<(String, String, Peripheral)>> {
        let adapter = get_adapter().await?;
        adapter
            .start_scan(ScanFilter {
                services: vec![SERVICE],
            })
            .await
            .context("Failed to start BLE scan")?;

        println!("Scanning for BLE devices ({SCAN_TIMEOUT:?})...");
        tokio::time::sleep(SCAN_TIMEOUT).await;
        adapter.stop_scan().await?;

        let mut found = Vec::new();
        for p in adapter.peripherals().await? {
            if let Some(props) = p.properties().await? {
                let name = props.local_name.unwrap_or_default();
                if name.starts_with(DEVICE_NAME_PREFIX) {
                    let addr = props.address.to_string();
                    found.push((name, addr, p));
                }
            }
        }
        Ok(found)
    }

    /// Run the full provisioning sequence on a discovered peripheral.
    pub async fn provision(
        peripheral: &Peripheral,
        config: &ProvisionConfig,
    ) -> Result<Option<String>> {
        println!("Connecting...");
        peripheral
            .connect()
            .await
            .context("Failed to connect to device")?;
        println!("Connected, discovering services...");
        peripheral
            .discover_services()
            .await
            .context("Service discovery failed")?;

        let chars = peripheral.characteristics();
        let write_char = find_characteristic(&chars, WRITE_CHAR)?;
        let notify_char = find_characteristic(&chars, NOTIFY_CHAR)?;

        // Subscribe to notifications
        peripheral
            .subscribe(&notify_char)
            .await
            .context("Failed to subscribe to notifications")?;
        let mut notifications = peripheral.notifications().await?;

        println!("Starting provisioning sequence...\n");

        let mut step = ProvisionStep::EnterAtMode;
        let mut step_num = 1u8;
        let mut mac_address: Option<String> = None;

        while step != ProvisionStep::Done {
            let packets = packets_for_step(step, config);
            let cmd_display = step.command(config).unwrap_or_default();

            // Mask password in display
            let display = if step == ProvisionStep::SetPassword && !config.wifi_password.is_empty()
            {
                format!(
                    "AT+WSKEY=WPA2PSK,AES,{}",
                    "*".repeat(config.wifi_password.len())
                )
            } else {
                cmd_display.clone()
            };

            let mut success = false;
            for attempt in 1..=STEP_RETRIES {
                if attempt > 1 {
                    println!("  Retry {attempt}/{STEP_RETRIES}...");
                } else {
                    println!("Step {step_num}: {step:?}");
                    println!(
                        "  Sending: \"{display}\" ({} chunk{})",
                        packets.len(),
                        if packets.len() == 1 { "" } else { "s" }
                    );
                }

                // Write all chunks
                for chunk in &packets {
                    peripheral
                        .write(&write_char, chunk, WriteType::WithResponse)
                        .await
                        .with_context(|| format!("BLE write failed at step {step:?}"))?;
                    // Small delay between chunks
                    if packets.len() > 1 {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }

                // Wait for notification response
                match tokio::time::timeout(STEP_TIMEOUT, notifications.next()).await {
                    Ok(Some(notification)) => {
                        let response = String::from_utf8_lossy(&notification.value);
                        let response = response.trim();
                        println!("  Response: \"{response}\"");

                        if step.is_success_response(response) {
                            if step == ProvisionStep::GetMac {
                                mac_address = ProvisionStep::parse_mac_response(response);
                                if let Some(ref mac) = mac_address {
                                    println!("  Device MAC: {mac}");
                                }
                            }
                            success = true;
                            break;
                        } else {
                            eprintln!("  Unexpected response, retrying...");
                        }
                    }
                    Ok(None) => {
                        eprintln!("  Notification stream ended");
                    }
                    Err(_) => {
                        eprintln!("  Timeout ({STEP_TIMEOUT:?}), no response");
                    }
                }
            }

            if !success {
                // Disconnect before returning error
                let _ = peripheral.disconnect().await;
                anyhow::bail!("Step {step:?} failed after {STEP_RETRIES} attempts");
            }

            println!();
            step = step.next();
            step_num += 1;

            if step != ProvisionStep::Done {
                tokio::time::sleep(INTER_STEP_DELAY).await;
            }
        }

        let _ = peripheral.disconnect().await;
        println!("Provisioning complete!");
        Ok(mac_address)
    }

    /// Scan for a device and provision it in one call.
    pub async fn scan_and_provision(config: &ProvisionConfig) -> Result<()> {
        let devices = scan().await?;

        if devices.is_empty() {
            anyhow::bail!("No GrillSense devices found via BLE scan");
        }

        println!("\nFound {} device(s):", devices.len());
        for (i, (name, addr, _)) in devices.iter().enumerate() {
            println!("  [{i}] {name} ({addr})");
        }
        println!();

        // Use first device
        let (ref name, ref addr, ref peripheral) = devices[0];
        println!("Provisioning {name} ({addr})...\n");
        print_provision_sequence(config);
        println!();

        let mac = provision(peripheral, config).await?;
        if let Some(mac) = mac {
            let device_id = crate::protocol::wifi_mac_to_device_id(&mac);
            println!("\nDevice MAC:  {mac}");
            println!("Device ID:   {device_id}");
            println!("Server:      {}:{}", config.server_host, config.server_port);
        }

        Ok(())
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
