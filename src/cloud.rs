#![allow(dead_code)]
/// Cloud API client for the GrillSense thermometer.
use anyhow::{Context, Result, bail};
use md5::{Digest, Md5};
use reqwest::Client;
use serde_json::json;

use crate::protocol::{self, ApiError, DeviceInfo, TempResult, TempUnit, UserInfo};

/// Client for the GrillSense cloud API.
#[derive(Clone)]
pub struct CloudClient {
    client: Client,
    base_url: String,
    token: Option<String>,
    device_mac: Option<String>,
}

impl CloudClient {
    /// Create a new cloud client.
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .danger_accept_invalid_certs(true) // matches the app's behavior
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self {
            client,
            base_url: protocol::CLOUD_BASE_URL.to_string(),
            token: None,
            device_mac: None,
        })
    }

    /// Set the auth token directly.
    pub fn set_token(&mut self, token: String) {
        self.token = Some(token);
    }

    /// Set the device MAC for temperature operations.
    /// Derives the cloud device ID from the WiFi MAC address.
    pub fn set_device_mac(&mut self, mac: String) {
        self.device_mac = Some(protocol::wifi_mac_to_device_id(&mac));
    }

    /// Set the device ID directly (already in cloud format like "02CC445566").
    pub fn set_device_id(&mut self, id: String) {
        self.device_mac = Some(id);
    }

    /// Get the currently set device MAC.
    pub fn device_mac(&self) -> Option<&str> {
        self.device_mac.as_deref()
    }

    /// Login with email and password. Stores the token on success.
    pub async fn login(&mut self, email: &str, password: &str) -> Result<UserInfo> {
        let pwd_hash = md5_hex(password);
        let url = format!("{}account/login", self.base_url);

        let resp = self
            .client
            .post(&url)
            .json(&json!({
                "email": email,
                "pwd": pwd_hash,
            }))
            .send()
            .await
            .context("Login request failed")?;

        let text = resp.text().await.context("Failed to read login response")?;

        // Check for error response
        if let Ok(err) = serde_json::from_str::<ApiError>(&text)
            && err.is_error()
        {
            bail!("Login failed: {}", err.description());
        }

        let user: UserInfo =
            serde_json::from_str(&text).context("Failed to parse login response")?;
        self.token = Some(user.token.clone());
        Ok(user)
    }

    /// List devices bound to the account.
    pub async fn list_devices(&self) -> Result<Vec<DeviceInfo>> {
        let token = self.token.as_ref().context("Not logged in")?;
        let url = format!("{}idev/list?token={}", self.base_url, token);

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("Device list request failed")?;

        let devices: Vec<DeviceInfo> = resp.json().await.context("Failed to parse device list")?;
        Ok(devices)
    }

    /// Bind a device to the account.
    pub async fn bind_device(&self, mac: &str) -> Result<()> {
        let token = self.token.as_ref().context("Not logged in")?;
        let url = format!("{}idev/bind?devmac={}&token={}", self.base_url, mac, token);

        let resp = self
            .client
            .post(&url)
            .json(&json!({ "alias": "Ezon WiFi BBQ" }))
            .send()
            .await
            .context("Bind device request failed")?;

        let text = resp.text().await?;
        if text != "{}"
            && let Ok(err) = serde_json::from_str::<ApiError>(&text)
            && let Some(code) = err.result
            && code != 0
        {
            bail!("Bind failed with code: {}", code);
        }
        Ok(())
    }

    /// Unbind a device from the account.
    pub async fn unbind_device(&self, mac: &str) -> Result<()> {
        let token = self.token.as_ref().context("Not logged in")?;
        let url = format!(
            "{}idev/unbind?devmac={}&token={}",
            self.base_url, mac, token
        );

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("Unbind device request failed")?;

        let text = resp.text().await?;
        if text != "{}"
            && let Ok(err) = serde_json::from_str::<ApiError>(&text)
            && let Some(code) = err.result
            && code != 0
        {
            bail!("Unbind failed with code: {}", code);
        }
        Ok(())
    }

    /// Get current temperature from the device.
    pub async fn get_temperature(&self) -> Result<TempResult> {
        let mac = self.device_mac.as_ref().context("No device MAC/ID set")?;
        let url = format!("{}thermo/temperature?devmac={}", self.base_url, mac);

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("Temperature request failed")?;

        let text = resp
            .text()
            .await
            .context("Failed to read temperature response")?;

        // Check for error response
        if let Ok(err) = serde_json::from_str::<ApiError>(&text)
            && err.is_error()
        {
            bail!("Cloud API error: {}", err.description());
        }

        let temp: TempResult =
            serde_json::from_str(&text).context("Failed to parse temperature response")?;
        Ok(temp)
    }

    /// Set alarm temperature for channel 1.
    pub async fn set_alarm_temp(&self, temp_celsius: f64) -> Result<()> {
        let mac = self.device_mac.as_ref().context("No device MAC set")?;
        let url = format!("{}thermo/set_alarm_temp?devmac={}", self.base_url, mac);

        self.client
            .post(&url)
            .json(&json!({ "alarm_temp_ch1": temp_celsius }))
            .send()
            .await
            .context("Set alarm temp request failed")?;

        Ok(())
    }

    /// Change temperature unit on the device.
    pub async fn change_unit(&self, unit: TempUnit) -> Result<()> {
        let mac = self.device_mac.as_ref().context("No device MAC set")?;
        let url = format!("{}thermo/change_unit?devmac={}", self.base_url, mac);

        self.client
            .post(&url)
            .json(&json!({ "unit": unit.as_str() }))
            .send()
            .await
            .context("Change unit request failed")?;

        Ok(())
    }
}

/// Compute MD5 hex digest of a string (matching the app's auth scheme).
fn md5_hex(input: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}

// Tiny hex encoder to avoid pulling in the `hex` crate.
mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        bytes.as_ref().iter().fold(String::new(), |mut s, b| {
            use std::fmt::Write;
            let _ = write!(s, "{b:02x}");
            s
        })
    }
}
