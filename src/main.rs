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

    /// Monitor temperature readings in real-time
    Monitor {
        /// Device MAC address (WiFi MAC from 'discover' command)
        #[arg(short, long)]
        mac: String,
        /// Auth token (optional — not needed for temperature reads)
        #[arg(short, long, default_value = "")]
        token: String,
        /// Polling interval in seconds
        #[arg(short, long, default_value = "3")]
        interval: u64,
        /// Show temperature in Fahrenheit
        #[arg(short = 'F', long)]
        fahrenheit: bool,
    },

    /// Set alarm temperature
    SetAlarm {
        /// Auth token (from login)
        #[arg(short, long)]
        token: String,
        /// Device MAC address
        #[arg(short, long)]
        mac: String,
        /// Alarm temperature in Celsius
        #[arg(short = 'T', long)]
        temp: f64,
    },

    /// Listen for UDP packets from the device (requires traffic redirection)
    UdpListen {
        /// UDP port to listen on
        #[arg(short, long, default_value = "17000")]
        port: u16,
    },

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
    DeviceInfo {
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

    /// Show BLE provisioning sequence (dry-run)
    BleProvision {
        /// WiFi SSID to configure
        #[arg(short = 's', long)]
        ssid: String,
        /// WiFi password
        #[arg(short = 'p', long)]
        wifi_password: String,
        /// Local server IP (omit to use cloud server)
        #[arg(short, long)]
        local_ip: Option<String>,
        /// Local server port
        #[arg(short = 'P', long, default_value = "17000")]
        local_port: u16,
    },

    /// Bridge temperature data to Home Assistant via MQTT
    HaBridge {
        /// Auth token (from login)
        #[arg(short, long)]
        token: String,
        /// Device MAC address
        #[arg(short, long)]
        mac: String,
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
        /// Polling interval in seconds
        #[arg(short, long, default_value = "3")]
        interval: u64,
    },

    /// Run UDP proxy: receive device data, forward to cloud + MQTT
    Proxy {
        /// UDP port to listen on
        #[arg(short, long, default_value = "17000")]
        port: u16,
        /// Forward packets to the cloud server (keeps official app working)
        #[arg(long, default_value = "true")]
        forward: bool,
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

    /// Show protocol information
    Protocol,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Login { email, password } => cmd_login(&email, &password).await,
        Commands::Devices { token } => cmd_devices(&token).await,
        Commands::Monitor {
            token,
            mac,
            interval,
            fahrenheit,
        } => cmd_monitor(&token, &mac, interval, fahrenheit).await,
        Commands::SetAlarm { token, mac, temp } => cmd_set_alarm(&token, &mac, temp).await,
        Commands::UdpListen { port } => udp::listen(port).await,
        Commands::Discover { ip, timeout: t } => cmd_discover(ip.as_deref(), t).await,
        Commands::DeviceInfo { ip } => cmd_device_info(&ip).await,
        Commands::Configure {
            ip,
            ssid,
            wifi_password,
            server,
            server_port,
            no_reboot,
        } => cmd_configure(&ip, &ssid, &wifi_password, &server, server_port, !no_reboot).await,
        Commands::BleProvision {
            ssid,
            wifi_password,
            local_ip,
            local_port,
        } => {
            cmd_ble_provision(&ssid, &wifi_password, local_ip.as_deref(), local_port);
            Ok(())
        }
        Commands::HaBridge {
            token,
            mac,
            mqtt_host,
            mqtt_port,
            mqtt_user,
            mqtt_pass,
            device_name,
            interval,
        } => {
            cmd_ha_bridge(
                &token,
                &mac,
                &mqtt_host,
                mqtt_port,
                mqtt_user,
                mqtt_pass,
                &device_name,
                interval,
            )
            .await
        }
        Commands::Protocol => {
            cmd_protocol();
            Ok(())
        }
        Commands::Proxy {
            port,
            forward,
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
                forward,
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
    }
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
    println!("Use this token with other commands:");
    println!("  grillsense devices --token {}", user.token);

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

    println!("{:<4} {:<20} {:<8} {:<16} {}", "ID", "MAC", "Online", "IP", "Location");
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
        println!(
            "  grillsense monitor --token {} --mac {}",
            token, devices[0].mac
        );
    }

    Ok(())
}

async fn cmd_monitor(token: &str, mac: &str, interval: u64, fahrenheit: bool) -> Result<()> {
    let mut client = cloud::CloudClient::new()?;
    client.set_token(token.to_string());
    client.set_device_mac(mac.to_string());

    let unit_label = if fahrenheit { "°F" } else { "°C" };
    let dev_id = client.device_mac().unwrap_or(mac);
    println!(
        "Monitoring device {} (cloud ID: {}, {}), Ctrl+C to stop...",
        mac, dev_id, unit_label
    );
    println!();

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

                print!("\r[{now}] {online} | {channels}    ");
                io::stdout().flush().context("flush stdout")?;
            }
            Err(e) => {
                consecutive_errors += 1;
                eprint!("\r[error] {} (attempt {})                ", e, consecutive_errors);
                io::stderr().flush().ok();
                if consecutive_errors >= 10 {
                    println!();
                    return Err(e.context("Too many consecutive errors"));
                }
            }
        }

        tokio::time::sleep(interval_dur).await;
    }
}

async fn cmd_set_alarm(token: &str, mac: &str, temp: f64) -> Result<()> {
    let mut client = cloud::CloudClient::new()?;
    client.set_token(token.to_string());
    client.set_device_mac(mac.to_string());

    client.set_alarm_temp(temp).await?;
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
        println!("  grillsense monitor --mac {}", dev.mac);
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
            println!("{:<16} {:<14} {:<12} {:<12}", dev.ip, dev.mac, dev.model, dev_id);
        }
        println!();
        if devices.len() == 1 {
            println!("Monitor this device:");
            println!("  grillsense monitor --mac {}", devices[0].mac);
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

fn cmd_protocol() {
    println!("GrillSense Protocol Summary");
    println!("===========================");
    println!();
    println!("Cloud API: {}", protocol::CLOUD_BASE_URL);
    println!("Cloud UDP: {}:{}", protocol::CLOUD_HOST, protocol::udp::CLOUD_PORT);
    println!();
    println!("BLE Service:  {}", protocol::ble::SERVICE_UUID);
    println!("BLE Notify:   {}", protocol::ble::NOTIFY_UUID);
    println!("BLE Write:    {}", protocol::ble::WRITE_UUID);
    println!("BLE Name:     {}*", protocol::ble::DEVICE_NAME_PREFIX);
    println!();
    println!("AP Mode SSID: {}", protocol::ap::DEFAULT_SSID);
    println!("AP Mode IP:   {}", protocol::ap::DEFAULT_IP);
    println!("AP Mode Port: {}", protocol::ap::DEFAULT_PORT);
    println!();
    println!("See PROTOCOL.md for full documentation.");
}

fn cmd_ble_provision(ssid: &str, wifi_password: &str, local_ip: Option<&str>, local_port: u16) {
    let config = if let Some(ip) = local_ip {
        ble::ProvisionConfig::local(
            ssid.to_string(),
            wifi_password.to_string(),
            ip.to_string(),
            local_port,
        )
    } else {
        ble::ProvisionConfig::cloud_default(ssid.to_string(), wifi_password.to_string())
    };

    ble::print_provision_sequence(&config);
    println!();
    if local_ip.is_some() {
        println!("NOTE: Device will be configured to send data to {}:{}", 
            config.server_host, config.server_port);
        println!("Run 'grillsense udp-listen' on that host to receive data.");
    } else {
        println!("NOTE: Device will be configured to send data to the cloud server.");
    }
    println!();
    println!("BLE provisioning requires a Bluetooth adapter and the btleplug runtime.");
    println!("This is a dry-run showing the command sequence.");
}

async fn cmd_ha_bridge(
    token: &str,
    mac: &str,
    mqtt_host: &str,
    mqtt_port: u16,
    mqtt_user: Option<String>,
    mqtt_pass: Option<String>,
    device_name: &str,
    interval: u64,
) -> Result<()> {
    let mut client = cloud::CloudClient::new()?;
    client.set_token(token.to_string());
    client.set_device_mac(mac.to_string());

    let config = mqtt::MqttHaConfig {
        broker_host: mqtt_host.to_string(),
        broker_port: mqtt_port,
        username: mqtt_user,
        password: mqtt_pass,
        device_name: device_name.to_string(),
        device_id: mac.to_string(),
        poll_interval: Duration::from_secs(interval),
    };

    println!("Starting Home Assistant MQTT bridge");
    println!("  Device:   {} ({})", device_name, mac);
    println!("  Broker:   {}:{}", mqtt_host, mqtt_port);
    println!("  Interval: {}s", interval);
    println!();

    mqtt::run_bridge(&config, &client).await
}

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
    println!("  grillsense configure --ip <device-ip> --ssid <ssid> -P <pass> --server <this-ip> --server-port {port}");
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
            if let Err(e) = mqtt_proxy_publisher(
                &mut packet_rx,
                &mac_id,
                &mqtt_host,
                mqtt_port,
                mqtt_user,
                mqtt_pass,
                &device_name,
            )
            .await
            {
                eprintln!("MQTT publisher error: {e}");
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

/// MQTT publisher that consumes device packets and publishes to HA.
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

    // MQTT CONNECT
    let connect = mqtt::build_mqtt_connect(
        &format!("grillsense_proxy_{device_id}"),
        mqtt_user.as_deref(),
        mqtt_pass.as_deref(),
        None,
    );
    stream.write_all(&connect).await?;

    let mut connack = [0u8; 4];
    stream.read_exact(&mut connack).await?;
    if connack[0] != 0x20 || connack[3] != 0x00 {
        anyhow::bail!("MQTT CONNACK failed (code: {})", connack[3]);
    }

    println!("[mqtt] Connected to {addr}");

    // Publish HA discovery for 6 channels
    let config = mqtt::MqttHaConfig {
        broker_host: mqtt_host.to_string(),
        broker_port: mqtt_port,
        username: mqtt_user,
        password: mqtt_pass,
        device_name: device_name.to_string(),
        device_id: device_id.to_string(),
        poll_interval: Duration::from_secs(1), // unused in proxy mode
    };

    for (topic, payload) in config.discovery_messages() {
        let packet = mqtt::build_mqtt_publish(&topic, payload.as_bytes(), true);
        stream.write_all(&packet).await?;
    }
    println!("[mqtt] Published HA discovery for 7 entities");

    // Mark online
    let avail = mqtt::build_mqtt_publish(&config.availability_topic(), b"online", true);
    stream.write_all(&avail).await?;

    // Process incoming device packets
    while let Some(pkt) = rx.recv().await {
        if pkt.direction != udp::PacketDirection::DeviceToCloud {
            continue;
        }
        if let Some(udp::ParsedData::Temperature(ref temp_pkt)) = pkt.parsed {
            // Convert to TempResult for the existing MQTT state format
            let temp_result = temp_pkt.to_temp_result();
            let payload = config.state_payload(&temp_result);

            let packet = mqtt::build_mqtt_publish(
                &config.state_topic(),
                payload.as_bytes(),
                false,
            );
            stream.write_all(&packet).await?;

            // Keepalive
            stream.write_all(&[0xC0, 0x00]).await?;
            let mut resp = [0u8; 64];
            let _ = tokio::time::timeout(
                Duration::from_millis(50),
                stream.read(&mut resp),
            )
            .await;
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

