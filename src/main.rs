mod ble;
mod cloud;
mod lan;
mod mqtt;
mod protocol;
mod udp;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::io::{self, Write};
use std::time::Duration;

#[derive(Parser)]
#[command(name = "grillsense", about = "GrillSense BBQ thermometer CLI tool")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Cloud API commands (requires internet)
    Cloud {
        #[command(subcommand)]
        command: CloudCommands,
    },

    /// Local device commands (LAN only, no cloud needed)
    Local {
        #[command(subcommand)]
        command: LocalCommands,
    },
}

// ---------- Cloud subcommands ----------

#[derive(Subcommand)]
enum CloudCommands {
    /// Login to the GrillSense cloud
    Login {
        /// Email address
        #[arg(short, long)]
        email: String,
        /// Password
        #[arg(short, long)]
        password: String,
    },

    /// List bound devices
    Devices {
        /// Auth token (from login)
        #[arg(short, long)]
        token: String,
    },

    /// Monitor temperature via cloud API polling
    Monitor {
        /// Device MAC address (WiFi MAC from 'local discover')
        #[arg(short, long)]
        mac: Option<String>,
        /// Auto-discover device MAC via LAN broadcast
        #[arg(long)]
        autodiscover: bool,
        /// Polling interval in seconds
        #[arg(short, long, default_value = "3")]
        interval: u64,
        /// Show temperature in Fahrenheit
        #[arg(short = 'F', long)]
        fahrenheit: bool,
        /// Also publish to MQTT for Home Assistant
        #[arg(long)]
        mqtt: bool,
        /// MQTT broker host
        #[arg(long, default_value = "localhost")]
        mqtt_host: String,
        /// MQTT broker port
        #[arg(long, default_value = "1883")]
        mqtt_port: u16,
        /// MQTT username
        #[arg(long)]
        mqtt_user: Option<String>,
        /// MQTT password
        #[arg(long)]
        mqtt_pass: Option<String>,
        /// Device name in Home Assistant
        #[arg(long, default_value = "BBQ Thermometer")]
        device_name: String,
    },

    /// Set alarm temperature on the cloud server
    SetAlarm {
        /// Device MAC address
        #[arg(short, long)]
        mac: String,
        /// Alarm temperature in Celsius
        #[arg(short = 'T', long)]
        temp: f64,
    },
}

// ---------- Local subcommands ----------

#[derive(Subcommand)]
enum LocalCommands {
    /// Discover HF modules on the local network
    Discover {
        /// Specific IP to probe (omit for broadcast scan)
        #[arg(short, long)]
        ip: Option<String>,
        /// Discovery timeout in seconds
        #[arg(short, long, default_value = "3")]
        timeout: u64,
    },

    /// Query device info via LAN AT commands
    Info {
        /// Device IP address
        #[arg(short, long)]
        ip: String,
    },

    /// Reconfigure device WiFi and server settings via LAN
    Configure {
        /// Device IP address
        #[arg(short, long)]
        ip: String,
        /// WiFi SSID
        #[arg(short, long)]
        ssid: String,
        /// WiFi password (empty for open networks)
        #[arg(short = 'P', long, default_value = "")]
        wifi_password: String,
        /// Server hostname or IP
        #[arg(long, default_value = "smartserver.emaxtime.cn")]
        server: String,
        /// Server UDP port
        #[arg(long, default_value = "17000")]
        server_port: u16,
        /// Skip reboot after configuration
        #[arg(long)]
        no_reboot: bool,
    },

    /// UDP proxy: intercept device traffic, forward to cloud + MQTT
    Proxy {
        /// UDP port to listen on
        #[arg(short, long, default_value = "17000")]
        port: u16,
        /// Disable forwarding to cloud (local-only mode)
        #[arg(long)]
        no_forward: bool,
        /// Also publish to MQTT for Home Assistant
        #[arg(long)]
        mqtt: bool,
        /// Device MAC address (for MQTT entity naming)
        #[arg(short, long)]
        mac: Option<String>,
        /// MQTT broker host
        #[arg(long, default_value = "localhost")]
        mqtt_host: String,
        /// MQTT broker port
        #[arg(long, default_value = "1883")]
        mqtt_port: u16,
        /// MQTT username
        #[arg(long)]
        mqtt_user: Option<String>,
        /// MQTT password
        #[arg(long)]
        mqtt_pass: Option<String>,
        /// Device name in Home Assistant
        #[arg(long, default_value = "BBQ Thermometer")]
        device_name: String,
    },

    /// Monitor temperature from local UDP packets (device must point here)
    Monitor {
        /// UDP port to listen on
        #[arg(short, long, default_value = "17000")]
        port: u16,
        /// Show temperature in Fahrenheit
        #[arg(short = 'F', long)]
        fahrenheit: bool,
        /// Also publish to MQTT for Home Assistant
        #[arg(long)]
        mqtt: bool,
        /// Device MAC address (for MQTT entity naming)
        #[arg(short, long)]
        mac: Option<String>,
        /// MQTT broker host
        #[arg(long, default_value = "localhost")]
        mqtt_host: String,
        /// MQTT broker port
        #[arg(long, default_value = "1883")]
        mqtt_port: u16,
        /// MQTT username
        #[arg(long)]
        mqtt_user: Option<String>,
        /// MQTT password
        #[arg(long)]
        mqtt_pass: Option<String>,
        /// Device name in Home Assistant
        #[arg(long, default_value = "BBQ Thermometer")]
        device_name: String,
    },

    /// Set device alarm by sending the alarm packet directly via UDP.
    SetAlarm {
        /// UDP port to listen on
        #[arg(short, long, default_value = "17000")]
        port: u16,
        /// Alarm threshold for channel 1 (°C)
        #[arg(long)]
        ch1: Option<f64>,
        /// Alarm threshold for channel 2 (°C)
        #[arg(long)]
        ch2: Option<f64>,
    },

    /// Scan for GrillSense devices via BLE (requires --features ble)
    #[cfg(feature = "ble")]
    Scan,

    /// Provision a GrillSense device via BLE (requires --features ble)
    #[cfg(feature = "ble")]
    Provision {
        /// WiFi SSID
        #[arg(short, long)]
        ssid: String,
        /// WiFi password (empty for open networks)
        #[arg(short = 'P', long, default_value = "")]
        wifi_password: String,
        /// Server hostname or IP to send temperature data to
        #[arg(long, default_value = "smartserver.emaxtime.cn")]
        server: String,
        /// Server UDP port
        #[arg(long, default_value = "17000")]
        server_port: u16,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Cloud { command } => match command {
            CloudCommands::Login { email, password } => cmd_login(&email, &password).await,
            CloudCommands::Devices { token } => cmd_devices(&token).await,
            CloudCommands::Monitor {
                mac,
                autodiscover,
                interval,
                fahrenheit,
                mqtt,
                mqtt_host,
                mqtt_port,
                mqtt_user,
                mqtt_pass,
                device_name,
            } => {
                let resolved_mac = resolve_device_mac(mac, autodiscover).await?;
                cmd_cloud_monitor(
                    &resolved_mac,
                    interval,
                    fahrenheit,
                    mqtt,
                    &mqtt_host,
                    mqtt_port,
                    mqtt_user,
                    mqtt_pass,
                    &device_name,
                )
                .await
            }
            CloudCommands::SetAlarm { mac, temp } => cmd_cloud_set_alarm(&mac, temp).await,
        },
        Commands::Local { command } => match command {
            LocalCommands::Discover { ip, timeout: t } => cmd_discover(ip.as_deref(), t).await,
            LocalCommands::Info { ip } => cmd_device_info(&ip).await,
            LocalCommands::Configure {
                ip,
                ssid,
                wifi_password,
                server,
                server_port,
                no_reboot,
            } => cmd_configure(&ip, &ssid, &wifi_password, &server, server_port, !no_reboot).await,
            LocalCommands::Proxy {
                port,
                no_forward,
                mqtt,
                mac,
                mqtt_host,
                mqtt_port,
                mqtt_user,
                mqtt_pass,
                device_name,
            } => {
                cmd_proxy(
                    port,
                    !no_forward,
                    mqtt,
                    mac,
                    &mqtt_host,
                    mqtt_port,
                    mqtt_user,
                    mqtt_pass,
                    &device_name,
                )
                .await
            }
            LocalCommands::Monitor {
                port,
                fahrenheit,
                mqtt,
                mac,
                mqtt_host,
                mqtt_port,
                mqtt_user,
                mqtt_pass,
                device_name,
            } => {
                cmd_local_monitor(
                    port,
                    fahrenheit,
                    mqtt,
                    mac,
                    &mqtt_host,
                    mqtt_port,
                    mqtt_user,
                    mqtt_pass,
                    &device_name,
                )
                .await
            }
            LocalCommands::SetAlarm { port, ch1, ch2 } => cmd_local_set_alarm(port, ch1, ch2).await,
            #[cfg(feature = "ble")]
            LocalCommands::Scan => cmd_ble_scan().await,
            #[cfg(feature = "ble")]
            LocalCommands::Provision {
                ssid,
                wifi_password,
                server,
                server_port,
            } => cmd_ble_provision(&ssid, &wifi_password, &server, server_port).await,
        },
    }
}

/// Resolve device MAC: use explicit value, autodiscover, or fail.
async fn resolve_device_mac(mac: Option<String>, autodiscover: bool) -> Result<String> {
    if let Some(mac) = mac
        && !mac.is_empty()
    {
        return Ok(mac);
    }
    if autodiscover {
        eprintln!("[discover] Auto-discovering device MAC via LAN broadcast...");
        let dev = lan::discover_with_retry(5, None).await?;
        eprintln!("[discover] Using MAC: {}", dev.mac);
        return Ok(dev.mac);
    }
    anyhow::bail!("Either --mac or --autodiscover is required")
}

async fn cmd_login(email: &str, password: &str) -> Result<()> {
    let mut client = cloud::CloudClient::new()?;
    let user = client.login(email, password).await?;

    println!("Login successful!");
    println!("  User ID:  {}", user.id);
    println!("  Nickname: {}", user.nickname);
    println!("  Email:    {}", user.email);
    println!("  Token:    {}", user.token);
    println!();
    println!("Use this token to list devices:");
    println!("  grillsense cloud devices --token {}", user.token);

    Ok(())
}

async fn cmd_devices(token: &str) -> Result<()> {
    let mut client = cloud::CloudClient::new()?;
    client.set_token(token.to_string());

    let devices = client.list_devices().await?;

    if devices.is_empty() {
        println!("No devices found.");
        return Ok(());
    }

    println!(
        "{:<4} {:<20} {:<8} {:<16} Location",
        "ID", "MAC", "Online", "IP"
    );
    println!("{}", "-".repeat(70));
    for dev in &devices {
        println!(
            "{:<4} {:<20} {:<8} {:<16} {}, {}",
            dev.id,
            dev.mac,
            if dev.isonline { "yes" } else { "no" },
            dev.ip,
            dev.city,
            dev.country,
        );
    }

    if devices.len() == 1 {
        println!();
        println!("Monitor this device:");
        println!("  grillsense cloud monitor --mac {}", devices[0].mac);
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn cmd_cloud_monitor(
    mac: &str,
    interval: u64,
    fahrenheit: bool,
    mqtt_enabled: bool,
    mqtt_host: &str,
    mqtt_port: u16,
    mqtt_user: Option<String>,
    mqtt_pass: Option<String>,
    device_name: &str,
) -> Result<()> {
    let mut client = cloud::CloudClient::new()?;
    client.set_device_mac(mac.to_string());

    // Use the derived device ID as canonical MQTT identifier (consistent with local mode)
    let dev_id = client.device_mac().unwrap_or(mac).to_string();
    let unit_label = if fahrenheit { "°F" } else { "°C" };
    println!(
        "Monitoring device {} (cloud ID: {}, {}), Ctrl+C to stop...",
        mac, dev_id, unit_label
    );
    if mqtt_enabled {
        println!("  MQTT: {}:{}", mqtt_host, mqtt_port);
    }
    println!();

    // Start MQTT bridge if enabled
    let mqtt_config = if mqtt_enabled {
        let config = mqtt::MqttHaConfig {
            broker_host: mqtt_host.to_string(),
            broker_port: mqtt_port,
            username: mqtt_user,
            password: mqtt_pass,
            device_name: device_name.to_string(),
            device_id: dev_id.clone(),
            poll_interval: Duration::from_secs(interval),
        };
        Some(config)
    } else {
        None
    };

    // If MQTT is enabled, delegate to the existing MQTT bridge
    if let Some(config) = mqtt_config {
        // Spawn a display task that also polls + prints to console
        let display_client = {
            let mut c = cloud::CloudClient::new()?;
            c.set_device_mac(mac.to_string());
            c
        };
        let interval_dur = Duration::from_secs(interval);
        tokio::spawn(async move {
            loop {
                if let Ok(temp) = display_client.get_temperature().await {
                    let online = if temp.online() { "online" } else { "OFFLINE" };
                    let now = chrono_lite_now();
                    let active = temp.active_channels();
                    let channels: String = if active.is_empty() {
                        "no probes connected".to_string()
                    } else {
                        active
                            .iter()
                            .map(|(ch, t)| {
                                let v = if fahrenheit {
                                    protocol::celsius_to_fahrenheit(*t)
                                } else {
                                    *t
                                };
                                let unit = if fahrenheit { "°F" } else { "°C" };
                                format!("CH{ch}: {v:.1}{unit}")
                            })
                            .collect::<Vec<_>>()
                            .join(" | ")
                    };
                    let age = format_age(&temp);
                    println!("[{now}] {online} | {channels}{age}");
                }
                tokio::time::sleep(interval_dur).await;
            }
        });

        mqtt::run_bridge_with_reconnect(&config, &client).await
    } else {
        // Console-only monitoring
        let interval_dur = Duration::from_secs(interval);
        let mut consecutive_errors = 0u32;

        loop {
            match client.get_temperature().await {
                Ok(temp) => {
                    consecutive_errors = 0;
                    let online = if temp.online() { "online" } else { "OFFLINE" };
                    let now = chrono_lite_now();

                    let active = temp.active_channels();
                    let channels: String = if active.is_empty() {
                        "no probes connected".to_string()
                    } else {
                        active
                            .iter()
                            .map(|(ch, t)| {
                                let v = if fahrenheit {
                                    protocol::celsius_to_fahrenheit(*t)
                                } else {
                                    *t
                                };
                                format!("CH{ch}: {v:.1}{unit_label}")
                            })
                            .collect::<Vec<_>>()
                            .join(" | ")
                    };

                    let age = format_age(&temp);
                    print!("\r\x1b[2K[{now}] {online} | {channels}{age}");
                    io::stdout().flush().context("flush stdout")?;
                }
                Err(e) => {
                    consecutive_errors += 1;
                    // Reduce log spam: verbose for first 10, then every 10th
                    if consecutive_errors <= 10 || consecutive_errors.is_multiple_of(10) {
                        eprint!("\r\x1b[2K[error] {} (×{})", e, consecutive_errors);
                        io::stderr().flush().ok();
                    }
                }
            }

            tokio::time::sleep(interval_dur).await;
        }
    }
}

async fn cmd_cloud_set_alarm(mac: &str, temp: f64) -> Result<()> {
    let mut client = cloud::CloudClient::new()?;
    client.set_device_mac(mac.to_string());

    client.set_alarm_temp(1, temp).await?;
    println!("Alarm temperature set to {:.1}°C for device {}", temp, mac);

    Ok(())
}

async fn cmd_discover(ip: Option<&str>, timeout_secs: u64) -> Result<()> {
    if let Some(ip) = ip {
        println!("Probing {ip}...");
        let dev = lan::discover_unicast(ip).await?;
        let dev_id = protocol::wifi_mac_to_device_id(&dev.mac);
        println!("Found: {} ({}) at {}", dev.model, dev.mac, dev.ip);
        println!("  Cloud device ID: {dev_id}");
        println!();
        println!("Monitor this device:");
        println!("  grillsense cloud monitor --mac {}", dev.mac);
        println!("  grillsense local monitor  (if device points here)");
    } else {
        println!("Scanning local network ({}s timeout)...", timeout_secs);
        let devices = lan::discover_broadcast(timeout_secs).await?;
        if devices.is_empty() {
            println!("No HF modules found.");
            return Ok(());
        }
        println!(
            "{:<16} {:<14} {:<12} {:<12}",
            "IP", "MAC", "Model", "Cloud ID"
        );
        println!("{}", "-".repeat(56));
        for dev in &devices {
            let dev_id = protocol::wifi_mac_to_device_id(&dev.mac);
            println!(
                "{:<16} {:<14} {:<12} {:<12}",
                dev.ip, dev.mac, dev.model, dev_id
            );
        }
        println!();
        if devices.len() == 1 {
            println!("Monitor this device:");
            println!("  grillsense cloud monitor --mac {}", devices[0].mac);
        }
    }
    Ok(())
}

async fn cmd_device_info(ip: &str) -> Result<()> {
    println!("Querying device at {ip}...");
    println!();

    let dev = lan::discover_unicast(ip).await?;
    println!("Device: {} ({})", dev.model, dev.mac);
    println!();

    let report = lan::query_device_info(ip).await?;
    print!("{report}");
    Ok(())
}

async fn cmd_configure(
    ip: &str,
    ssid: &str,
    wifi_password: &str,
    server: &str,
    server_port: u16,
    reboot: bool,
) -> Result<()> {
    println!("Configuring device at {ip}...");
    println!("  SSID:   {ssid}");
    println!("  Server: {server}:{server_port}");
    println!();

    lan::configure_device(ip, ssid, wifi_password, server, server_port, reboot).await?;

    println!();
    if reboot {
        println!("Device is rebooting. It should rejoin the network in ~10 seconds.");
    } else {
        println!("Configuration saved. Reboot the device to apply (AT+Z or power cycle).");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn cmd_proxy(
    port: u16,
    forward: bool,
    mqtt_enabled: bool,
    mac: Option<String>,
    mqtt_host: &str,
    mqtt_port: u16,
    mqtt_user: Option<String>,
    mqtt_pass: Option<String>,
    device_name: &str,
) -> Result<()> {
    use tokio::sync::mpsc;

    let cloud_addr = udp::resolve_cloud_addr().await?;

    println!("GrillSense UDP Proxy");
    println!("====================");
    println!("  Listen:  0.0.0.0:{port}");
    println!("  Cloud:   {cloud_addr} (forward: {forward})");
    println!("  MQTT:    {mqtt_enabled}");
    println!();
    println!("Configure device to send here:");
    println!(
        "  grillsense local configure --ip <device-ip> --ssid <ssid> -P <pass> --server <this-ip> --server-port {port}"
    );
    println!();

    let (packet_tx, mut packet_rx) = mpsc::channel::<udp::DevicePacket>(64);

    // Start MQTT publisher task if enabled
    if mqtt_enabled {
        let mac_id = mac.clone().unwrap_or_else(|| "unknown".to_string());
        let mqtt_host = mqtt_host.to_string();
        let mqtt_user = mqtt_user.clone();
        let mqtt_pass = mqtt_pass.clone();
        let device_name = device_name.to_string();
        tokio::spawn(async move {
            let mut attempts = 0u32;
            loop {
                match mqtt_proxy_publisher(
                    &mut packet_rx,
                    &mac_id,
                    &mqtt_host,
                    mqtt_port,
                    mqtt_user.clone(),
                    mqtt_pass.clone(),
                    &device_name,
                )
                .await
                {
                    Ok(()) => break,
                    Err(e) => {
                        attempts += 1;
                        let delay = (5 * attempts).min(60);
                        eprintln!("[mqtt] Publisher error: {e}");
                        eprintln!("[mqtt] Reconnecting in {delay}s (attempt {attempts})...");
                        tokio::time::sleep(Duration::from_secs(delay.into())).await;
                    }
                }
            }
        });
    } else {
        // Drain the channel so the proxy doesn't block
        tokio::spawn(async move {
            while let Some(pkt) = packet_rx.recv().await {
                // Log parsed data even without MQTT
                if let Some(udp::ParsedData::Temperature(ref pkt)) = pkt.parsed {
                    let active = pkt.active_channels();
                    let temps: Vec<String> = active
                        .iter()
                        .map(|(ch, t)| format!("CH{ch}: {t:.1}°C"))
                        .collect();
                    if !temps.is_empty() {
                        println!("  [parsed] {}", temps.join(" | "));
                    }
                }
            }
        });
    }

    udp::run_proxy(udp::ProxyConfig {
        listen_port: port,
        cloud_addr,
        forward_to_cloud: forward,
        packet_tx: Some(packet_tx),
    })
    .await
}

#[allow(clippy::too_many_arguments)]
async fn cmd_local_monitor(
    port: u16,
    fahrenheit: bool,
    mqtt_enabled: bool,
    mac: Option<String>,
    mqtt_host: &str,
    mqtt_port: u16,
    mqtt_user: Option<String>,
    mqtt_pass: Option<String>,
    device_name: &str,
) -> Result<()> {
    println!("GrillSense Local Monitor");
    println!("========================");
    println!("  Listen: 0.0.0.0:{port}");
    println!("  Unit:   {}", if fahrenheit { "°F" } else { "°C" });
    println!("  MQTT:   {mqtt_enabled}");
    println!();

    let sock = std::sync::Arc::new(
        tokio::net::UdpSocket::bind(("0.0.0.0", port))
            .await
            .with_context(|| format!("Failed to bind UDP port {port}"))?,
    );

    println!("Waiting for device packets...");
    println!();

    let mut buf = [0u8; 256];

    // If MQTT is enabled but no --mac, learn device ID from first packet
    let mac_id = if mqtt_enabled && mac.is_none() {
        let (len, addr) = sock.recv_from(&mut buf).await?;
        let data = &buf[..len];

        // Echo back so device stays connected
        if let Some(echo) = build_echo(data) {
            let _ = sock.send_to(&echo, addr).await;
        }

        if let Some(id_bytes) = protocol::udp::parse_device_id_bytes(data) {
            let id: String = id_bytes.iter().map(|b| format!("{b:02X}")).collect();
            println!("[mqtt] Learned device ID from first packet: {id}");
            Some(id)
        } else {
            println!(
                "[mqtt] Warning: Could not parse device ID from first packet, using 'unknown'"
            );
            None
        }
    } else {
        mac.clone()
    };

    // Start MQTT publisher task if enabled
    let mqtt_tx = if mqtt_enabled {
        let mac_id = mac_id.unwrap_or_else(|| "unknown".to_string());
        let (tx, mut rx) = tokio::sync::mpsc::channel::<udp::DevicePacket>(64);
        let mqtt_host = mqtt_host.to_string();
        let mqtt_user = mqtt_user.clone();
        let mqtt_pass = mqtt_pass.clone();
        let device_name = device_name.to_string();
        tokio::spawn(async move {
            let mut attempts = 0u32;
            loop {
                match mqtt_proxy_publisher(
                    &mut rx,
                    &mac_id,
                    &mqtt_host,
                    mqtt_port,
                    mqtt_user.clone(),
                    mqtt_pass.clone(),
                    &device_name,
                )
                .await
                {
                    Ok(()) => break,
                    Err(e) => {
                        attempts += 1;
                        let delay = (5 * attempts).min(60);
                        eprintln!("[mqtt] Publisher error: {e}");
                        eprintln!("[mqtt] Reconnecting in {delay}s (attempt {attempts})...");
                        tokio::time::sleep(Duration::from_secs(delay.into())).await;
                    }
                }
            }
        });
        Some(tx)
    } else {
        None
    };

    loop {
        let (len, addr) = sock.recv_from(&mut buf).await?;
        let data = &buf[..len];

        // Echo back so device stays connected
        if let Some(echo) = build_echo(data) {
            let _ = sock.send_to(&echo, addr).await;
        }

        // Feed MQTT publisher
        if let Some(ref tx) = mqtt_tx {
            let parsed = if let Some(pkt) = protocol::udp::TempPacket::parse(data) {
                Some(udp::ParsedData::Temperature(pkt))
            } else if let Some((ch, temp_c)) = protocol::udp::parse_alarm_packet(data) {
                Some(udp::ParsedData::Alarm {
                    channel: ch,
                    temp_c,
                })
            } else {
                None
            };
            let _ = tx.try_send(udp::DevicePacket {
                source: addr,
                _raw: data.to_vec(),
                direction: udp::PacketDirection::DeviceToCloud,
                parsed,
            });
        }

        // Display temperature
        if let Some(pkt) = protocol::udp::TempPacket::parse(data) {
            let active = pkt.active_channels();
            if active.is_empty() {
                continue;
            }
            let temps: Vec<String> = active
                .iter()
                .map(|(ch, t)| {
                    let val = if fahrenheit {
                        *t * 9.0 / 5.0 + 32.0
                    } else {
                        *t
                    };
                    let unit = if fahrenheit { "°F" } else { "°C" };
                    format!("CH{ch}: {val:.1}{unit}")
                })
                .collect();
            let ts = chrono_lite_now();
            println!("[{ts}] {addr} — {}", temps.join(" | "));
            io::stdout().flush().ok();
        }
    }
}

async fn cmd_local_set_alarm(
    port: u16,
    ch1_threshold: Option<f64>,
    ch2_threshold: Option<f64>,
) -> Result<()> {
    if ch1_threshold.is_none() && ch2_threshold.is_none() {
        anyhow::bail!("At least one threshold required: --ch1 <temp> and/or --ch2 <temp>");
    }

    println!("GrillSense Local Set-Alarm");
    println!("==========================");
    if let Some(t) = ch1_threshold {
        println!("  CH1 alarm: {t:.1}°C");
    }
    if let Some(t) = ch2_threshold {
        println!("  CH2 alarm: {t:.1}°C");
    }
    println!("  Listen:    0.0.0.0:{port}");
    println!();

    // Bind to the port the device is sending to
    let sock = tokio::net::UdpSocket::bind(("0.0.0.0", port))
        .await
        .with_context(|| format!("Failed to bind UDP port {port}"))?;

    println!("Waiting for a device packet to learn its address and ID...");

    // Wait for a packet from the device to learn its address and device ID
    let mut buf = [0u8; 256];
    let (len, device_addr) = sock.recv_from(&mut buf).await?;
    let data = &buf[..len];

    let device_id_bytes = protocol::udp::parse_device_id_bytes(data)
        .context("Could not parse device ID from packet")?;
    let device_id: String = device_id_bytes.iter().map(|b| format!("{b:02X}")).collect();

    println!("  Device found: {device_id} at {device_addr}");
    println!();

    // Echo back to keep device happy
    if let Some(echo) = build_echo(data) {
        let _ = sock.send_to(&echo, device_addr).await;
    }

    // Build and send alarm packet(s)
    let mut sent = 0;
    if let Some(temp) = ch1_threshold {
        let pkt = protocol::udp::build_alarm_packet(&device_id_bytes, 1, temp);
        sock.send_to(&pkt, device_addr).await?;
        println!("  ✓ Sent CH1 alarm: {temp:.1}°C → {device_addr}");
        println!("    Packet: {}", udp::hex_encode(&pkt));
        sent += 1;
    }
    if let Some(temp) = ch2_threshold {
        let pkt = protocol::udp::build_alarm_packet(&device_id_bytes, 2, temp);
        sock.send_to(&pkt, device_addr).await?;
        println!("  ✓ Sent CH2 alarm: {temp:.1}°C → {device_addr}");
        println!("    Packet: {}", udp::hex_encode(&pkt));
        sent += 1;
    }

    println!();
    println!("Sent {sent} alarm packet(s). Listening for confirmation...");
    println!();

    // Continue listening briefly to echo back and see if device behavior changes
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let timeout = tokio::time::sleep_until(deadline);
        tokio::select! {
            _ = timeout => break,
            result = sock.recv_from(&mut buf) => {
                let (len, addr) = result?;
                let data = &buf[..len];

                // Check for alarm acknowledgment
                if let Some((ch, temp)) = protocol::udp::parse_alarm_packet(data) {
                    println!("  [alarm ack] CH{ch} alarm = {temp:.1}°C from {addr}");
                } else if let Some(pkt) = protocol::udp::TempPacket::parse(data) {
                    let active = pkt.active_channels();
                    let temps: Vec<String> = active.iter().map(|(ch, t)| format!("CH{ch}={t:.1}°C")).collect();
                    print!("\r  [temp] {}  ", temps.join(" | "));
                    io::stdout().flush().ok();
                    // Echo back
                    if let Some(echo) = build_echo(data) {
                        let _ = sock.send_to(&echo, addr).await;
                    }
                } else {
                    println!("  [unknown] {} bytes from {addr}: {}", data.len(), udp::hex_encode(data));
                }
            }
        }
    }

    println!();
    println!("Done. The device alarm should now be set.");
    Ok(())
}

/// Build an echo response — delegates to protocol::udp::build_echo.
fn build_echo(data: &[u8]) -> Option<Vec<u8>> {
    protocol::udp::build_echo(data)
}

/// MQTT publisher that consumes device packets, publishes to HA,
/// and subscribes to alarm command topics to send alarm packets to the device.
async fn mqtt_proxy_publisher(
    rx: &mut tokio::sync::mpsc::Receiver<udp::DevicePacket>,
    device_id: &str,
    mqtt_host: &str,
    mqtt_port: u16,
    mqtt_user: Option<String>,
    mqtt_pass: Option<String>,
    device_name: &str,
) -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    let addr = format!("{mqtt_host}:{mqtt_port}");
    let mut stream = TcpStream::connect(&addr)
        .await
        .with_context(|| format!("Failed to connect to MQTT broker at {addr}"))?;

    // MQTT CONNECT with LWT for availability
    let config = mqtt::MqttHaConfig {
        broker_host: mqtt_host.to_string(),
        broker_port: mqtt_port,
        username: mqtt_user.clone(),
        password: mqtt_pass.clone(),
        device_name: device_name.to_string(),
        device_id: device_id.to_string(),
        poll_interval: Duration::from_secs(1),
    };

    let connect = mqtt::build_mqtt_connect(
        &format!("grillsense_proxy_{device_id}"),
        mqtt_user.as_deref(),
        mqtt_pass.as_deref(),
        Some((&config.availability_topic(), "offline")),
    );
    stream.write_all(&connect).await?;

    let mut connack = [0u8; 4];
    stream.read_exact(&mut connack).await?;
    if connack[0] != 0x20 || connack[3] != 0x00 {
        anyhow::bail!("MQTT CONNACK failed (code: {})", connack[3]);
    }

    println!("[mqtt] Connected to {addr}");

    // Publish HA discovery (6 probes + online + 2 alarm number entities)
    let discovery_msgs = config.discovery_messages();
    let entity_count = discovery_msgs.len();
    for (topic, payload) in &discovery_msgs {
        let packet = mqtt::build_mqtt_publish(topic, payload.as_bytes(), true);
        stream.write_all(&packet).await?;
    }
    println!("[mqtt] Published HA discovery for {entity_count} entities");

    // Mark online
    let avail = mqtt::build_mqtt_publish(&config.availability_topic(), b"online", true);
    stream.write_all(&avail).await?;

    // Subscribe to alarm command topics
    let alarm_ch1_topic = config.alarm_command_topic(1);
    let alarm_ch2_topic = config.alarm_command_topic(2);
    let subscribe =
        mqtt::build_mqtt_subscribe(&[alarm_ch1_topic.as_str(), alarm_ch2_topic.as_str()], 1);
    stream.write_all(&subscribe).await?;
    println!("[mqtt] Subscribed to alarm commands: {alarm_ch1_topic}, {alarm_ch2_topic}");

    // UDP socket for sending alarm packets to the device
    let alarm_sock = tokio::net::UdpSocket::bind("0.0.0.0:0").await?;
    let mut device_addr: Option<std::net::SocketAddr> = None;
    let mut device_id_bytes: Option<[u8; 5]> = None;

    // Track current alarm setpoints for state publishing
    let mut alarm_ch1: f64 = 0.0;
    let mut alarm_ch2: f64 = 0.0;

    // Split TCP stream for concurrent read/write
    let (reader, mut writer) = stream.into_split();
    let reader = std::sync::Arc::new(tokio::sync::Mutex::new(reader));

    // Spawn a task to read from MQTT broker (alarm commands, PINGRESP, SUBACK)
    let (mqtt_cmd_tx, mut mqtt_cmd_rx) = tokio::sync::mpsc::channel::<(u8, f64)>(16);
    let mqtt_reader = reader.clone();
    tokio::spawn(async move {
        let mut buf = [0u8; 1024];
        let mut partial = Vec::new();
        let mut reader = mqtt_reader.lock().await;
        loop {
            match reader.read(&mut buf).await {
                Ok(0) => break, // connection closed
                Ok(n) => {
                    partial.extend_from_slice(&buf[..n]);
                    // Process complete packets from the buffer
                    while let Some(pkt_len) = mqtt::mqtt_packet_len(&partial) {
                        if partial.len() < pkt_len {
                            break; // need more data
                        }
                        let pkt_data: Vec<u8> = partial.drain(..pkt_len).collect();
                        if let Some((topic, payload, _)) = mqtt::parse_incoming_publish(&pkt_data)
                            && let Ok(text) = std::str::from_utf8(&payload)
                            && let Ok(temp) = text.trim().parse::<f64>()
                        {
                            let channel = if topic.contains("alarm_ch2") { 2 } else { 1 };
                            let _ = mqtt_cmd_tx.try_send((channel, temp));
                        }
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Main loop: process device packets, MQTT alarm commands, and keepalive
    let mut ping_interval = tokio::time::interval(Duration::from_secs(30));
    ping_interval.tick().await; // consume the immediate first tick

    loop {
        tokio::select! {
            pkt = rx.recv() => {
                let Some(pkt) = pkt else { break };

                // Learn device address from incoming packets
                if pkt.direction == udp::PacketDirection::DeviceToCloud {
                    device_addr = Some(pkt.source);

                    // Learn device ID bytes from first temp packet
                    if device_id_bytes.is_none()
                        && let Some(udp::ParsedData::Temperature(ref temp_pkt)) = pkt.parsed
                            && let Some(id) = protocol::udp::parse_device_id_bytes(&temp_pkt.raw) {
                                device_id_bytes = Some(id);
                            }
                }

                // Track alarm setpoints from cloud→device alarm packets
                if let Some(udp::ParsedData::Alarm { channel, temp_c }) = &pkt.parsed {
                    match channel {
                        1 => alarm_ch1 = *temp_c,
                        2 => alarm_ch2 = *temp_c,
                        _ => {}
                    }
                    println!("[mqtt] Alarm CH{channel} updated to {temp_c:.1}°C (from cloud)");
                }

                if pkt.direction != udp::PacketDirection::DeviceToCloud {
                    continue;
                }

                if let Some(udp::ParsedData::Temperature(ref temp_pkt)) = pkt.parsed {
                    let temp_result = temp_pkt.to_temp_result();
                    let state = serde_json::to_string(&serde_json::json!({
                        "temperature_ch1": temp_result.temperature_ch1,
                        "temperature_ch2": temp_result.temperature_ch2,
                        "temperature_ch3": temp_result.temperature_ch3,
                        "temperature_ch4": temp_result.temperature_ch4,
                        "temperature_ch5": temp_result.temperature_ch5,
                        "temperature_ch6": temp_result.temperature_ch6,
                        "is_online": temp_result.online(),
                        "alarm_ch1": alarm_ch1,
                        "alarm_ch2": alarm_ch2,
                    })).unwrap();

                    let packet = mqtt::build_mqtt_publish(
                        &config.state_topic(), state.as_bytes(), false,
                    );
                    writer.write_all(&packet).await?;
                }
            }

            _ = ping_interval.tick() => {
                writer.write_all(&[0xC0, 0x00]).await?;
            }

            cmd = mqtt_cmd_rx.recv() => {
                let Some((channel, temp_c)) = cmd else {
                    // Reader task exited — MQTT connection lost
                    anyhow::bail!("MQTT reader task exited — connection lost");
                };

                println!("[mqtt] Alarm command received: CH{channel} = {temp_c:.1}°C");

                if let (Some(addr), Some(id_bytes)) = (device_addr, device_id_bytes) {
                    let alarm_pkt = protocol::udp::build_alarm_packet(
                        &id_bytes, channel, temp_c,
                    );
                    match alarm_sock.send_to(&alarm_pkt, addr).await {
                        Ok(_) => {
                            println!("[mqtt] Sent alarm CH{channel}={temp_c:.1}°C to {addr}");
                            match channel {
                                1 => alarm_ch1 = temp_c,
                                2 => alarm_ch2 = temp_c,
                                _ => {}
                            }
                        }
                        Err(e) => eprintln!("[mqtt] Failed to send alarm to device: {e}"),
                    }
                } else {
                    eprintln!("[mqtt] Cannot send alarm: device address not yet learned");
                }
            }
        }
    }

    Ok(())
}

/// Simple timestamp without pulling in chrono.
fn chrono_lite_now() -> String {
    use std::time::SystemTime;
    let dur = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();
    let hours = (secs / 3600) % 24;
    let mins = (secs / 60) % 60;
    let s = secs % 60;
    format!("{hours:02}:{mins:02}:{s:02}")
}

/// Format data age for display. Returns empty string if fresh (< 30s).
fn format_age(temp: &protocol::TempResult) -> String {
    match temp.age_secs() {
        Some(age) if age >= 60 => {
            let mins = age / 60;
            let secs = age % 60;
            if mins >= 60 {
                format!(" (stale: {}h{}m)", mins / 60, mins % 60)
            } else {
                format!(" (stale: {mins}m{secs:02}s)")
            }
        }
        Some(age) if age >= 30 => format!(" (age: {age}s)"),
        _ => String::new(),
    }
}

// ---------- BLE commands ----------

#[cfg(feature = "ble")]
async fn cmd_ble_scan() -> Result<()> {
    let devices = ble::runtime::scan().await?;

    if devices.is_empty() {
        println!("No GrillSense devices found via BLE.");
        println!("Make sure:");
        println!("  - The device is powered on and NOT connected to WiFi");
        println!("  - You are within BLE range (~10m)");
        println!("  - Bluetooth is enabled on this machine");
        return Ok(());
    }

    println!("\nFound {} device(s):", devices.len());
    for (name, addr, _) in &devices {
        println!("  {name} ({addr})");
    }
    Ok(())
}

#[cfg(feature = "ble")]
async fn cmd_ble_provision(
    ssid: &str,
    wifi_password: &str,
    server: &str,
    server_port: u16,
) -> Result<()> {
    let config = if server == protocol::CLOUD_HOST {
        ble::ProvisionConfig::cloud_default(ssid.to_string(), wifi_password.to_string())
    } else {
        ble::ProvisionConfig::local(
            ssid.to_string(),
            wifi_password.to_string(),
            server.to_string(),
            server_port,
        )
    };

    println!("GrillSense BLE Provisioning");
    println!("===========================");
    println!("  SSID:   {ssid}");
    println!("  Server: {server}:{server_port}");
    println!();

    ble::runtime::scan_and_provision(&config).await?;

    println!();
    println!("Provisioning complete. Settings saved to flash.");
    println!();
    println!("⚠️  If the device was in AP mode, you must POWER CYCLE it.");
    println!("   (AT+Z cannot exit AP mode — this is a hardware limitation.)");
    println!();
    println!("Temperature data will be sent to {server}:{server_port}.");
    Ok(())
}
