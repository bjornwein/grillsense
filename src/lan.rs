/// LAN discovery and AT command interface for Hi-Flying WiFi modules.
///
/// The HF-LPT230 module exposes a UDP-based AT command interface on port 48899.
/// This works in both AP mode (10.10.100.254) and STA mode (on the local network).
use anyhow::{Context, Result, bail};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::time::timeout;

use crate::protocol::lan;

/// Information returned by the discovery handshake.
#[derive(Debug, Clone)]
pub struct DeviceDiscovery {
    pub ip: String,
    pub mac: String,
    pub model: String,
    pub _source: SocketAddr,
}

/// Discover HF modules on the local network via broadcast.
///
/// Sends the discovery magic string to the broadcast address on port 48899
/// and collects responses within the timeout period.
pub async fn discover_broadcast(timeout_secs: u64) -> Result<Vec<DeviceDiscovery>> {
    let socket = UdpSocket::bind("0.0.0.0:0")
        .await
        .context("Failed to bind UDP socket for discovery")?;
    socket
        .set_broadcast(true)
        .context("Failed to enable broadcast")?;

    let dest: SocketAddr = format!("255.255.255.255:{}", lan::DISCOVERY_PORT)
        .parse()
        .unwrap();

    socket
        .send_to(lan::DISCOVERY_MAGIC.as_bytes(), dest)
        .await
        .context("Failed to send discovery broadcast")?;

    let mut devices = Vec::new();
    let mut buf = [0u8; 512];
    let deadline = Duration::from_secs(timeout_secs);

    while let Ok(Ok((len, src))) = timeout(deadline, socket.recv_from(&mut buf)).await {
        let text = String::from_utf8_lossy(&buf[..len]);
        if let Some(dev) = parse_discovery_response(&text, src) {
            // Deduplicate by MAC
            if !devices.iter().any(|d: &DeviceDiscovery| d.mac == dev.mac) {
                devices.push(dev);
            }
        }
    }

    Ok(devices)
}

/// Discover a specific device by IP address.
pub async fn discover_unicast(ip: &str) -> Result<DeviceDiscovery> {
    let socket = UdpSocket::bind("0.0.0.0:0")
        .await
        .context("Failed to bind UDP socket")?;

    let dest: SocketAddr = format!("{ip}:{}", lan::DISCOVERY_PORT)
        .parse()
        .context("Invalid IP address")?;

    socket
        .send_to(lan::DISCOVERY_MAGIC.as_bytes(), dest)
        .await
        .context("Failed to send discovery packet")?;

    let mut buf = [0u8; 512];
    let (len, src) = timeout(Duration::from_secs(3), socket.recv_from(&mut buf))
        .await
        .context("Discovery timed out")?
        .context("Failed to receive response")?;

    let text = String::from_utf8_lossy(&buf[..len]);
    parse_discovery_response(&text, src).context("Invalid discovery response")
}

fn parse_discovery_response(text: &str, src: SocketAddr) -> Option<DeviceDiscovery> {
    let parts: Vec<&str> = text.trim().split(',').collect();
    if parts.len() >= 3 {
        Some(DeviceDiscovery {
            ip: parts[0].to_string(),
            mac: parts[1].to_string(),
            model: parts[2].to_string(),
            _source: src,
        })
    } else {
        None
    }
}

/// Send a single AT command to the device and return the response.
///
/// This performs the full handshake: discovery → +ok → command.
pub async fn send_at_command(ip: &str, command: &str) -> Result<String> {
    let results = send_at_commands(ip, &[command]).await?;
    results.into_iter().next().context("No response")
}

/// Discover the first GrillSense device on the network, retrying until found.
///
/// Sends broadcast discovery every `interval` seconds. If `max_attempts` is `None`,
/// retries forever (suitable for unmonitored services). Log frequency is reduced
/// after the first 12 attempts to avoid spam.
pub async fn discover_with_retry(
    interval_secs: u64,
    max_attempts: Option<u32>,
) -> Result<DeviceDiscovery> {
    let mut attempt = 0u32;
    loop {
        attempt += 1;

        if let Some(max) = max_attempts
            && attempt > max
        {
            anyhow::bail!("No GrillSense device found after {max} discovery attempts");
        }

        // Reduce log spam: verbose for first 12 attempts, then every 12th
        let verbose = attempt <= 12 || attempt.is_multiple_of(12);
        if verbose {
            match max_attempts {
                Some(max) => eprintln!("[discover] Broadcast scan attempt {attempt}/{max}..."),
                None => eprintln!("[discover] Broadcast scan attempt {attempt}..."),
            }
        }

        match discover_broadcast(interval_secs).await {
            Ok(devices) if !devices.is_empty() => {
                let dev = devices.into_iter().next().unwrap();
                eprintln!("[discover] Found {} ({}) at {}", dev.model, dev.mac, dev.ip);
                return Ok(dev);
            }
            Ok(_) => {
                if verbose {
                    eprintln!("[discover] No devices found, retrying in {interval_secs}s...");
                }
            }
            Err(e) => {
                if verbose {
                    eprintln!("[discover] Scan error: {e}");
                }
            }
        }

        tokio::time::sleep(Duration::from_secs(interval_secs)).await;
    }
}

/// Send multiple AT commands in a single session.
///
/// Opens one UDP session: discovery → +ok → cmd1 → cmd2 → ...
pub async fn send_at_commands(ip: &str, commands: &[&str]) -> Result<Vec<String>> {
    let socket = UdpSocket::bind("0.0.0.0:0")
        .await
        .context("Failed to bind UDP socket")?;

    let dest: SocketAddr = format!("{ip}:{}", lan::DISCOVERY_PORT)
        .parse()
        .context("Invalid IP address")?;

    let mut buf = [0u8; 1024];

    // Step 1: Discovery handshake
    socket
        .send_to(lan::DISCOVERY_MAGIC.as_bytes(), dest)
        .await?;
    let _len = timeout(Duration::from_secs(3), socket.recv_from(&mut buf))
        .await
        .context("Discovery timed out")?
        .context("recv failed")?;

    // Step 2: Enter AT mode
    tokio::time::sleep(Duration::from_millis(200)).await;
    socket.send_to(lan::AT_MODE_ENTER.as_bytes(), dest).await?;
    // Try to consume any AT mode acknowledgment (some firmwares send one)
    let _ = timeout(Duration::from_millis(500), socket.recv_from(&mut buf)).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Step 3: Send each command
    let mut results = Vec::new();
    for cmd in commands {
        let cmd_with_crlf = format!("{cmd}\r\n");
        socket.send_to(cmd_with_crlf.as_bytes(), dest).await?;

        let (len, _) = timeout(Duration::from_secs(3), socket.recv_from(&mut buf))
            .await
            .context("AT command timed out")?
            .context("recv failed")?;

        let response = String::from_utf8_lossy(&buf[..len]).trim().to_string();
        results.push(response);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    Ok(results)
}

/// Query all device settings and return them as a formatted report.
pub async fn query_device_info(ip: &str) -> Result<String> {
    let labels = [
        "MAC",
        "SSID",
        "WiFi Key",
        "Network",
        "Mode",
        "UART",
        "Firmware",
        "TCP Timeout",
    ];
    let commands = [
        "AT+WSMAC",
        "AT+WSSSID",
        "AT+WSKEY",
        "AT+NETP",
        "AT+WMODE",
        "AT+UART",
        "AT+VER",
        "AT+TCPTO",
    ];

    let responses = send_at_commands(ip, &commands).await?;

    let mut report = String::new();
    for (label, resp) in labels.iter().zip(responses.iter()) {
        let value = resp.strip_prefix("+ok=").unwrap_or(resp).to_string();
        // Mask WiFi password in output
        let display = if *label == "WiFi Key" {
            let parts: Vec<&str> = value.splitn(3, ',').collect();
            if parts.len() == 3 {
                format!("{},{},****", parts[0], parts[1])
            } else {
                value
            }
        } else {
            value
        };
        report.push_str(&format!("  {label:<12}: {display}\n"));
    }

    Ok(report)
}

/// Reconfigure the device's WiFi and server settings.
pub async fn configure_device(
    ip: &str,
    ssid: &str,
    password: &str,
    server_host: &str,
    server_port: u16,
    reboot: bool,
) -> Result<()> {
    let key_cmd = if password.is_empty() {
        "AT+WSKEY=OPEN,NONE,".to_string()
    } else {
        format!("AT+WSKEY=WPA2PSK,AES,{password}")
    };
    let netp_cmd = format!("AT+NETP=UDP,CLIENT,{server_port},{server_host}");

    let labels = [
        "Set SSID",
        "Set WiFi key",
        "Set server",
        "Set STA mode",
        "Save to flash",
    ];
    let commands = [
        format!("AT+WSSSID={ssid}"),
        key_cmd,
        netp_cmd,
        "AT+WMODE=STA".to_string(),
        "AT+CFGTF".to_string(),
    ];
    let cmd_refs: Vec<&str> = commands.iter().map(|s| s.as_str()).collect();

    let responses = send_at_commands(ip, &cmd_refs).await?;

    for (label, resp) in labels.iter().zip(responses.iter()) {
        if resp.contains("+ok") {
            println!("  ✓ {label}");
        } else {
            bail!("{label} failed: {resp}");
        }
    }

    if reboot {
        println!("  Rebooting device...");
        let _ = send_at_command(ip, "AT+Z").await;
        println!("  ✓ Reboot command sent");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddrV4};

    #[test]
    fn test_parse_discovery_response() {
        let src = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 145), 48899));
        let resp = parse_discovery_response("192.168.1.50,AABBCC445566,HF-LPT230", src);
        assert!(resp.is_some());
        let dev = resp.unwrap();
        assert_eq!(dev.ip, "192.168.1.50");
        assert_eq!(dev.mac, "AABBCC445566");
        assert_eq!(dev.model, "HF-LPT230");
    }

    #[test]
    fn test_parse_discovery_response_short() {
        let src = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 1), 48899));
        let resp = parse_discovery_response("garbage", src);
        assert!(resp.is_none());
    }
}
