#![allow(dead_code)]
/// UDP listener for intercepting device-to-cloud temperature packets.
///
/// The device sends UDP datagrams to smartserver.emaxtime.cn:17000.
/// By redirecting DNS or routing, these packets can be captured locally.
use anyhow::{Context, Result};
use std::net::SocketAddr;
use tokio::net::UdpSocket;

/// Parsed temperature packet from the device.
#[derive(Debug, Clone)]
pub struct UdpTempPacket {
    pub source: SocketAddr,
    pub raw: Vec<u8>,
    pub raw_hex: String,
    pub raw_ascii: String,
}

/// Listen for UDP packets on the specified port.
///
/// This binds to `0.0.0.0:<port>` and prints every received packet.
/// To receive device data, redirect the device's cloud traffic here
/// (e.g., via DNS override for smartserver.emaxtime.cn or iptables DNAT).
pub async fn listen(port: u16) -> Result<()> {
    let addr = format!("0.0.0.0:{port}");
    let socket = UdpSocket::bind(&addr)
        .await
        .with_context(|| format!("Failed to bind UDP socket on {addr}"))?;

    println!("UDP listener bound on {addr}");
    println!("Waiting for packets... (redirect device traffic here)");
    println!();
    println!("Tip: Add to /etc/hosts or DNS:");
    println!("  <your-ip>  smartserver.emaxtime.cn");
    println!();

    let mut buf = [0u8; 4096];
    let mut packet_count: u64 = 0;

    loop {
        let (len, src) = socket
            .recv_from(&mut buf)
            .await
            .context("Failed to receive UDP packet")?;

        packet_count += 1;
        let data = &buf[..len];

        let packet = UdpTempPacket {
            source: src,
            raw: data.to_vec(),
            raw_hex: hex_encode(data),
            raw_ascii: lossy_ascii(data),
        };

        println!("--- Packet #{packet_count} from {src} ({len} bytes) ---");
        println!("  Hex:   {}", packet.raw_hex);
        println!("  ASCII: {}", packet.raw_ascii);

        // Attempt to parse known patterns
        if let Some(parsed) = try_parse_temp_packet(data) {
            println!("  >> Parsed: {parsed}");
        }

        println!();

        // Echo back an acknowledgement so the device doesn't retry
        let _ = socket.send_to(b"+ok\r\n", src).await;
    }
}

/// Attempt to parse temperature data from the raw packet.
///
/// The exact format is not fully known yet — this function uses heuristics
/// from the cloud API response format and common IoT packet patterns.
fn try_parse_temp_packet(data: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(data).ok()?;

    // Try JSON format (unlikely from device, but possible)
    if text.contains("temperature") {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
            let ch1 = v.get("temperature_ch1").and_then(|v| v.as_f64());
            let ch2 = v.get("temperature_ch2").and_then(|v| v.as_f64());
            if let (Some(c1), Some(c2)) = (ch1, ch2) {
                return Some(format!("JSON: ch1={c1:.1}°C, ch2={c2:.1}°C"));
            }
        }
    }

    // Try comma-separated numeric values (common in cheap IoT devices)
    let parts: Vec<&str> = text.trim().split(',').collect();
    if parts.len() >= 2 {
        if let (Ok(v1), Ok(v2)) = (parts[0].trim().parse::<f64>(), parts[1].trim().parse::<f64>())
        {
            if (-50.0..500.0).contains(&v1) && (-50.0..500.0).contains(&v2) {
                return Some(format!("CSV: ch1={v1:.1}°C, ch2={v2:.1}°C"));
            }
        }
    }

    // If data is short binary, try interpreting as 16-bit big-endian temps
    if data.len() >= 4 && data.len() <= 32 && !text.chars().all(|c| c.is_ascii_graphic()) {
        let t1 = i16::from_be_bytes([data[0], data[1]]) as f64 / 10.0;
        let t2 = i16::from_be_bytes([data[2], data[3]]) as f64 / 10.0;
        if (-50.0..500.0).contains(&t1) && (-50.0..500.0).contains(&t2) {
            return Some(format!(
                "Binary(BE/10): ch1={t1:.1}°C, ch2={t2:.1}°C (speculative)"
            ));
        }
    }

    None
}

fn hex_encode(data: &[u8]) -> String {
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
