/// UDP proxy that intercepts device-to-cloud traffic.
///
/// Sits between the device and the cloud server, forwarding all packets
/// in both directions while extracting temperature data locally.
///
/// Architecture:
/// ```text
/// Device ──UDP──► [Local Proxy :17000] ──UDP──► Cloud :17000
///                      │          ▲
///                      │          │ (cloud responses forwarded back)
///                      ▼
///                   parse + optional MQTT
/// ```
use anyhow::{Context, Result};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

use crate::protocol;

/// Parsed data from a device packet.
#[derive(Debug, Clone)]
pub struct DevicePacket {
    pub source: SocketAddr,
    pub raw: Vec<u8>,
    pub direction: PacketDirection,
    pub parsed: Option<ParsedData>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketDirection {
    DeviceToCloud,
    CloudToDevice,
}

impl std::fmt::Display for PacketDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DeviceToCloud => write!(f, "device→cloud"),
            Self::CloudToDevice => write!(f, "cloud→device"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ParsedData {
    Temperature(protocol::udp::TempPacket),
    Csv(Vec<String>),
    Unknown,
}

/// Configuration for the UDP proxy.
pub struct ProxyConfig {
    /// Local port to listen on.
    pub listen_port: u16,
    /// Cloud server address (ip:port) to forward to.
    pub cloud_addr: SocketAddr,
    /// Whether to forward packets to the cloud.
    pub forward_to_cloud: bool,
    /// Channel to send parsed packets for MQTT or other consumers.
    pub packet_tx: Option<mpsc::Sender<DevicePacket>>,
}

/// Run the UDP proxy.
///
/// Binds on `0.0.0.0:<listen_port>`, forwards device packets to the cloud,
/// forwards cloud responses back to the device, and sends parsed packets
/// through the channel for MQTT publishing.
pub async fn run_proxy(config: ProxyConfig) -> Result<()> {
    let listen_addr = format!("0.0.0.0:{}", config.listen_port);
    let listen_socket = Arc::new(
        UdpSocket::bind(&listen_addr)
            .await
            .with_context(|| format!("Failed to bind on {listen_addr}"))?,
    );

    println!("UDP proxy listening on {listen_addr}");
    if config.forward_to_cloud {
        println!("Forwarding to cloud: {}", config.cloud_addr);
    } else {
        println!("Cloud forwarding DISABLED (local-only mode)");
    }

    // Socket for talking to the cloud
    let cloud_socket = Arc::new(
        UdpSocket::bind("0.0.0.0:0")
            .await
            .context("Failed to bind cloud-side UDP socket")?,
    );

    let mut buf = [0u8; 4096];
    let mut cloud_buf = [0u8; 4096];
    let mut packet_count: u64 = 0;
    // Track the device's address so we can forward cloud responses back
    let mut device_addr: Option<SocketAddr> = None;

    println!("Waiting for device packets...");
    println!();

    loop {
        tokio::select! {
            // Packets from the device (or any client)
            result = listen_socket.recv_from(&mut buf) => {
                let (len, src) = result.context("recv_from failed")?;
                let data = &buf[..len];
                packet_count += 1;
                device_addr = Some(src);

                let parsed = try_parse(data);
                print_packet(packet_count, PacketDirection::DeviceToCloud, src, data, &parsed);

                // Send to consumer (MQTT, etc.)
                if let Some(ref tx) = config.packet_tx {
                    let pkt = DevicePacket {
                        source: src,
                        raw: data.to_vec(),
                        direction: PacketDirection::DeviceToCloud,
                        parsed: parsed.clone(),
                    };
                    let _ = tx.try_send(pkt);
                }

                // Forward to cloud
                if config.forward_to_cloud {
                    if let Err(e) = cloud_socket.send_to(data, config.cloud_addr).await {
                        eprintln!("  [!] Cloud forward failed: {e}");
                    }
                }
            }

            // Responses from the cloud
            result = cloud_socket.recv_from(&mut cloud_buf) => {
                let (len, src) = result.context("cloud recv_from failed")?;
                let data = &cloud_buf[..len];

                let parsed = try_parse(data);
                print_packet(packet_count, PacketDirection::CloudToDevice, src, data, &parsed);

                // Send to consumer
                if let Some(ref tx) = config.packet_tx {
                    let pkt = DevicePacket {
                        source: src,
                        raw: data.to_vec(),
                        direction: PacketDirection::CloudToDevice,
                        parsed: try_parse(data),
                    };
                    let _ = tx.try_send(pkt);
                }

                // Forward back to the device
                if let Some(dev) = device_addr {
                    if let Err(e) = listen_socket.send_to(data, dev).await {
                        eprintln!("  [!] Device forward failed: {e}");
                    }
                }
            }
        }
    }
}

/// Resolve the cloud server address to a SocketAddr.
pub async fn resolve_cloud_addr() -> Result<SocketAddr> {
    use tokio::net::lookup_host;
    let host_port = format!("{}:{}", protocol::CLOUD_HOST, protocol::udp::CLOUD_PORT);
    let addr = lookup_host(&host_port)
        .await
        .with_context(|| format!("Failed to resolve {host_port}"))?
        .next()
        .with_context(|| format!("No addresses for {host_port}"))?;
    Ok(addr)
}

fn print_packet(
    num: u64,
    dir: PacketDirection,
    src: SocketAddr,
    data: &[u8],
    parsed: &Option<ParsedData>,
) {
    let hex = hex_encode(data);
    let ascii = lossy_ascii(data);
    println!("--- #{num} {dir} from {src} ({} bytes) ---", data.len());
    println!("  Hex:   {hex}");
    println!("  ASCII: {ascii}");
    match parsed {
        Some(ParsedData::Temperature(pkt)) => {
            let active = pkt.active_channels();
            if active.is_empty() {
                println!("  >> Temp: no probes (device {})", pkt.device_id);
            } else {
                let temps: String = active
                    .iter()
                    .map(|(ch, t)| format!("CH{ch}: {t:.1}°C"))
                    .collect::<Vec<_>>()
                    .join(" | ");
                println!("  >> Temp: {temps} (device {})", pkt.device_id);
            }
        }
        Some(ParsedData::Csv(parts)) => {
            println!("  >> CSV: {}", parts.join(", "));
        }
        Some(ParsedData::Unknown) => {
            println!("  >> (unknown format)");
        }
        None => {}
    }
    println!();
}

/// Attempt to parse device data from a raw packet.
fn try_parse(data: &[u8]) -> Option<ParsedData> {
    // Try the known binary temperature packet format first
    if let Some(pkt) = protocol::udp::TempPacket::parse(data) {
        return Some(ParsedData::Temperature(pkt));
    }

    // Fallback: try text-based formats for unknown packet types
    if let Ok(text) = std::str::from_utf8(data) {
        let trimmed = text.trim();
        let parts: Vec<&str> = trimmed.split(',').collect();
        if parts.len() >= 2 {
            return Some(ParsedData::Csv(
                parts.iter().map(|s| s.to_string()).collect(),
            ));
        }
    }

    Some(ParsedData::Unknown)
}

pub fn hex_encode(data: &[u8]) -> String {
    data.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn lossy_ascii(data: &[u8]) -> String {
    data.iter()
        .map(|&b| {
            if b.is_ascii_graphic() || b == b' ' {
                b as char
            } else {
                '.'
            }
        })
        .collect()
}
