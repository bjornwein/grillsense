# GrillSense / Dangrill WiFi BBQ Thermometer Protocol

Reverse-engineered from the Android app `com.bbq.ap` v1.1.9 (GrillSense).

## Device Overview

The device is a WiFi-enabled BBQ thermometer with 2 temperature probe channels.
It uses a **Hi-Flying HF-A11** WiFi module controlled via AT commands.

- **Manufacturer**: Ezon (branded as Dangrill, GrillSense, and others)
- **Connectivity**: BLE 4.0+ (provisioning) + WiFi 802.11 b/g/n (data)
- **Cloud server**: `smartserver.emaxtime.cn`
- **BLE advertisement name**: `Thermo-typ*` (prefix match)

## Communication Architecture

```
┌──────────┐   BLE (setup)   ┌──────────┐  UDP:17000  ┌─────────────────────┐
│  Phone   │◄───────────────►│  Device  │────────────►│  Cloud Server       │
│  (App)   │                 │ (HF-A11) │             │  smartserver.       │
└────┬─────┘                 └──────────┘             │  emaxtime.cn        │
     │                                                └──────────┬──────────┘
     │          HTTPS REST API (V1.0/)                           │
     └───────────────────────────────────────────────────────────┘
```

1. **BLE** is used only during initial setup to configure WiFi credentials
2. **WiFi** — device connects to home network and sends temperature via **UDP** to the cloud
3. **Cloud REST API** — the app polls for temperature data over HTTPS

## BLE Provisioning Protocol

### GATT Service & Characteristics

| Name    | UUID                                   | Properties    |
|---------|----------------------------------------|---------------|
| Service | `0000fff0-0000-1000-8000-00805f9b34fb` | —             |
| Notify  | `0000fff1-0000-1000-8000-00805f9b34fb` | Notify        |
| Write   | `0000fff3-0000-1000-8000-00805f9b34fb` | Write         |

### BLE Packet Framing

Commands are sent as BLE GATT writes. Since BLE has a 20-byte MTU, longer
commands are split into chunks:

```
Byte 0: Sequence number (ASCII '1'=0x31, '2'=0x32, '3'=0x33)
Byte 1: Total chunk count (1, 2, or 3)
Bytes 2..19: Payload data (up to 18 bytes per chunk)
```

- Payloads ≤ 18 bytes: 1 chunk
- Payloads 19–36 bytes: 2 chunks (first 18 bytes + remainder)
- Payloads 37–54 bytes: 3 chunks
- AT commands (except steps 1 and 2) get `\r\n` appended before chunking

### Provisioning Sequence

The BLE provisioning sends AT commands to the HF-A11 WiFi module through
the BLE characteristic. Each step waits for a notify response.

| Step | Command Sent                                        | Expected Response | Notes                          |
|------|-----------------------------------------------------|-------------------|--------------------------------|
| 1    | `+++`                                               | `a` or `+ERR`     | Enter AT command mode          |
| 2    | `a`                                                 | `+ok`             | Confirm AT mode                |
| 3    | `AT+WSMAC`                                          | `+ok=<MAC>`       | Get device MAC address         |
| 4    | `AT+WSSSID=<ssid>`                                  | `+ok`             | Set WiFi SSID                  |
| 5    | `AT+WSKEY=OPEN,NONE,<password>`                     | `+ok`             | Set WiFi password              |
| 6    | `AT+NETP=UDP,CLIENT,17000,smartserver.emaxtime.cn`  | `+ok`             | Set cloud server endpoint      |
| 7    | `AT+WMODE=STA`                                      | `+ok`             | Set WiFi to station mode       |
| 8    | `AT+Z`                                              | `+ok`             | Reboot/apply settings          |

**Response format**: All responses start with `+ok` on success or `+ERR` on failure.
MAC address is returned as `+ok=<MAC>` in step 3.

### Timeout & Retry

- Each step has a 3-second timeout
- Up to 3 retries per step before aborting

## WiFi AP Mode Configuration

When not yet provisioned (or in reset state), the device runs as a WiFi AP:

- **SSID**: `LivingSmart`
- **IP**: `10.10.100.254`
- **UDP Port**: `8800`

### AP Mode Command Sequence

Uses UDP datagrams to `10.10.100.254:8800`:

| Step | Command Sent           | Expected Response | Notes                     |
|------|------------------------|-------------------|---------------------------|
| 1    | `HF-A11ASSISTHREAD`   | `+ok`             | Handshake / discovery     |
| 2    | `+ok`                 | —                 | Acknowledge, then unicast |
| 3    | `AT+NETP=UDP,CLIENT,10000,47.52.149.125\r\n` | `+ok` | Set cloud endpoint   |
| 4    | `AT+WSSSID=<ssid>\r\n`| `+ok`             | Set WiFi SSID             |
| 5    | `AT+WSKEY=<pwd>\r\n`  | `+ok`             | Set WiFi password         |
| 6    | `AT+WMODE=STA\r\n`    | `+ok`             | Set station mode          |
| 7    | `AT+Z\r\n`            | `+ok`             | Reboot                    |

### AP Mode BLE-bridged Commands

When using BLE-to-WiFi bridge mode, commands are prefixed with `LSD_WIFI:`:

| Command                                            | Purpose                    |
|----------------------------------------------------|----------------------------|
| `LSD_WIFI`                                         | Initiate BLE-WiFi bridge   |
| `LSD_WIFI:AT+WSSSID=<ssid>`                       | Set WiFi SSID              |
| `LSD_WIFI:AT+WSKEY=<pwd>`                          | Set WiFi password          |
| `LSD_WIFI:AT+RELD`                                 | Reload/restart             |
| `LSD_WIFI:AT+WALK`                                 | Walk/scan WiFi             |
| `LSD_WIFI:AT+NETP=TCP,SERVER,8899,10.10.100.254\r\n` | Set network parameters |
| `LSD_WIFI:AT+WSMAC`                                | Get MAC address            |

## Cloud REST API

**Base URL**: `https://smartserver.emaxtime.cn/V1.0/`

All requests use JSON content type. Authentication uses email + MD5-hashed password.

### Authentication

#### Login
```
POST /V1.0/account/login
Content-Type: application/json

{
    "email": "user@example.com",
    "pwd": "<MD5 hex of password>"
}

Response (success):
{
    "id": 123,
    "nickname": "User",
    "email": "user@example.com",
    "token": "<auth_token>",
    "sex": 0
}

Response (error):
{
    "result": <error_code>,
    "info": "<error_message>"
}
```

#### Register
```
POST /V1.0/account/reg
Content-Type: application/json

{
    "email": "user@example.com",
    "nickname": "User",
    "pwd": "<MD5 hex of password>"
}

Response: { "result": 0 }  (0 = success)
```

#### Reset Password
```
POST /V1.0/account/resetpwd
Content-Type: application/json

{
    "email": "user@example.com",
    "pwd": "<MD5 hex of new password>"
}

Response: { "result": 0 }
```

### Device Management

#### List Devices
```
GET /V1.0/idev/list?token=<auth_token>

Response:
[
    {
        "id": 1,
        "mac": "AA:BB:CC:DD:EE:FF",
        "city": "Stockholm",
        "ip": "1.2.3.4",
        "country": "SE",
        "isonline": true,
        "serial": 12345,
        "timezone": "Europe/Stockholm",
        "type": 1
    }
]
```

#### Bind Device
```
POST /V1.0/idev/bind?devmac=<mac>&token=<auth_token>
Content-Type: application/json

{
    "alias": "Ezon WiFi BBQ"
}

Response: {} (success) or { "result": <error_code> }
```

#### Unbind Device
```
GET /V1.0/idev/unbind?devmac=<mac>&token=<auth_token>

Response: {} (success) or { "result": <error_code> }
```

### Temperature Data

#### Get Temperature
```
GET /V1.0/thermo/temperature?devmac=<mac>

Response:
{
    "is_online": true,
    "temperature_ch1": 72.5,
    "temperature_ch2": 0.0
}

Error Response:
{
    "error_code": "102",
    "error_message": "设备不存在"
}
```

- `temperature_ch1`: Probe 1 temperature in **Celsius**
- `temperature_ch2`: Probe 2 temperature in **Celsius**
- `is_online`: Whether the device is currently connected
- Values of `0.0` typically mean no probe connected

**Known error codes:**

| Code | Message (Chinese)  | Meaning                |
|------|--------------------|------------------------|
| 102  | 设备不存在          | Device does not exist  |

**Note:** The `devmac` parameter format may vary. Use `idev/list` to see the
exact MAC format the server expects for your device.

#### Set Alarm Temperature
```
POST /V1.0/thermo/set_alarm_temp?devmac=<mac>
Content-Type: application/json

{
    "alarm_temp_ch1": 75.0
}
```

#### Change Temperature Unit
```
POST /V1.0/thermo/change_unit?devmac=<mac>
Content-Type: application/json

{
    "unit": "C"
}
```

Unit values: `"C"` (Celsius) or `"F"` (Fahrenheit).

## Device-to-Cloud UDP Protocol

The device sends temperature data via **UDP** to the cloud server on port **17000**.

- Destination: `smartserver.emaxtime.cn:17000`
- Alternative endpoint found in AP mode: `47.52.149.125:10000`
- Packet format: **Unknown** — needs traffic capture to determine exact format
- Polling interval in app: every 3 seconds

## Temperature Conversion

```
Fahrenheit = round((Celsius × 9 / 5) + 32)
Celsius = round((Fahrenheit - 32) × 5 / 9)
```

## Default Meat Temperature Presets (°C)

| Meat       | Full | Seven-tenths | Semi | Third |
|------------|------|--------------|------|-------|
| Beef       | 77   | 71           | 66   | 63    |
| Pork       | 77   | 71           | —    | —     |
| Veal       | 74   | 66           | 63   | —     |
| Lamb       | 77   | 71           | 63   | —     |
| Chicken    | 74   | —            | —    | —     |
| Fish       | 63   | —            | —    | —     |
| Turkey     | 74   | —            | —    | —     |
| Hamburger  | 71   | —            | —    | —     |

## Device Types

The app references two device type identifiers:
- `G001` (G_0)
- `G002` (G_1)

## Security Notes

- The cloud API uses HTTPS but the app **disables certificate validation** (trusts all certs)
- Passwords are hashed with plain MD5 (no salt)
- The BLE provisioning sends WiFi credentials in **cleartext** over BLE
- The device MAC address is used as the primary device identifier (no additional auth for temperature reads)
