mod cloud;
mod protocol;

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
        /// Auth token (from login)
        #[arg(short, long)]
        token: String,
        /// Device MAC address
        #[arg(short, long)]
        mac: String,
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
        Commands::Protocol => {
            cmd_protocol();
            Ok(())
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

    println!(
        "Monitoring device {} ({}), Ctrl+C to stop...",
        mac,
        if fahrenheit { "°F" } else { "°C" }
    );
    println!();

    let interval_dur = Duration::from_secs(interval);
    let mut consecutive_errors = 0u32;

    loop {
        match client.get_temperature().await {
            Ok(temp) => {
                consecutive_errors = 0;
                let (ch1, ch2, unit) = if fahrenheit {
                    (
                        protocol::celsius_to_fahrenheit(temp.temperature_ch1),
                        protocol::celsius_to_fahrenheit(temp.temperature_ch2),
                        "°F",
                    )
                } else {
                    (temp.temperature_ch1, temp.temperature_ch2, "°C")
                };

                let online = if temp.is_online { "online" } else { "OFFLINE" };
                let now = chrono_lite_now();

                // Use \r to overwrite the line for a clean live display
                print!(
                    "\r[{}] {} | CH1: {:6.1}{} | CH2: {:6.1}{}    ",
                    now, online, ch1, unit, ch2, unit,
                );
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

