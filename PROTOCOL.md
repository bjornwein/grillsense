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

## LAN Discovery & AT Command Interface

The HF-LPT230 WiFi module exposes an AT command interface over **UDP port 48899**,
accessible from the local network in both AP and STA modes.

### Discovery Protocol

Send the magic string `HF-A11ASSISTHREAD` as a UDP datagram to port 48899
(unicast or broadcast). The module responds with:

```
<ip>,<mac>,<model>
```

Example response: `192.168.1.50,AABBCC445566,HF-LPT230`

**Note:** The MAC is returned **uppercase, no separators**.

### AT Command Mode over UDP

After discovery, enter AT command mode by sending `+ok` and then issue
AT commands. Each command session requires the full handshake:

1. Send `HF-A11ASSISTHREAD` → receive `<ip>,<mac>,<model>`
2. Send `+ok` → enters AT mode
3. Send `AT+<CMD>\r\n` → receive `+ok` or `+ok=<value>`

### Verified AT Commands

Confirmed working on HF-LPT230 firmware v4.12.17:

| Command              | Description                      | Example Response                       |
|----------------------|----------------------------------|----------------------------------------|
| `AT+WSMAC`           | Get WiFi MAC address             | `+ok=AABBCC445566`                     |
| `AT+WSSSID`          | Get/set WiFi SSID                | `+ok=MyNetwork`                       |
| `AT+WSKEY`           | Get/set WiFi security & password | `+ok=WPA2PSK,AES,<password>`           |
| `AT+NETP`            | Get/set network endpoint         | `+ok=UDP,Client,17000,smartserver...`  |
| `AT+WMODE`           | Get/set WiFi mode (STA/AP)       | `+ok=STA`                              |
| `AT+UART`            | Get/set UART settings            | `+ok=9600,8,1,None,NFC`               |
| `AT+VER`             | Get firmware version             | `+ok=4.12.17 (2019-01-09 10:30 1M)`   |
| `AT+TCPTO`           | Get/set TCP timeout              | `+ok=300`                              |
| `AT+Z`               | Reboot (preserves settings)      | `+ok`                                  |
| `AT+RELD`            | **Factory reset** (wipes config) | `+ok=rebooting...`                     |

**⚠️ WARNING**: `AT+RELD` performs a factory reset, NOT a simple reboot. Use `AT+Z` to reboot.

### WiFi Key Formats

The `AT+WSKEY` command uses the format: `<auth>,<encryption>,<password>`

| Auth Type  | Encryption | Example                                  |
|------------|------------|------------------------------------------|
| `OPEN`     | `NONE`     | `AT+WSKEY=OPEN,NONE,`                    |
| `WPA2PSK`  | `AES`      | `AT+WSKEY=WPA2PSK,AES,mypassword`        |
| `WPA2PSK`  | `TKIPAES`  | `AT+WSKEY=WPA2PSK,TKIPAES,mypassword`    |

### Hardware Details

- **WiFi module**: Hi-Flying HF-LPT230 (similar to HF-A11 family)
- **Firmware**: v4.12.17 (2019-01-09)
- **UART to MCU**: 9600 baud, 8N1, NFC flow control
- **WiFi MAC**: `AA:BB:CC:44:55:66` (Hi-Flying OUI `AA:BB:CC`)
- **AP MAC**: `AA:BB:CC:44:55:67` (one byte higher than STA MAC)

## WiFi AP Mode Configuration

When not yet provisioned (or after factory reset), the device runs as a WiFi AP:

- **SSID**: `LivingSmart`
- **Security**: Open (no password)
- **IP**: `10.10.100.254`
- **UDP Port**: `48899` (AT commands) / `8800` (legacy)

### AP Mode Reconfiguration

Connect to the `LivingSmart` WiFi network, then send AT commands to
`10.10.100.254:48899`:

| Step | Command Sent                                        | Expected Response     | Notes                          |
|------|-----------------------------------------------------|-----------------------|--------------------------------|
| 1    | `HF-A11ASSISTHREAD`                                | `10.10.100.254,...`   | Discovery handshake            |
| 2    | `+ok`                                              | —                     | Enter AT mode                  |
| 3    | `AT+WSSSID=<ssid>`                                 | `+ok`                 | Set WiFi SSID                  |
| 4    | `AT+WSKEY=WPA2PSK,AES,<password>`                  | `+ok`                 | Set WiFi key                   |
| 5    | `AT+NETP=UDP,CLIENT,17000,smartserver.emaxtime.cn` | `+ok`                 | Set cloud endpoint             |
| 6    | `AT+WMODE=STA`                                     | `+ok`                 | Set station mode               |
| 7    | `AT+Z`                                             | `+ok`                 | Reboot (preserves settings)    |

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

#### Device ID Derivation (Critical)

The `devmac` parameter in API calls is **NOT** the WiFi MAC address. It is a
derived identifier computed from the WiFi MAC:

```
WiFi MAC:     AABBCC445566
              ^^^^          ← Remove first 4 hex chars (first 2 bytes)
Remaining:        CC445566
Prepend "02": 02CC445566   ← This is the device ID
```

**Formula**: `devmac = "02" + wifi_mac[4:]`

This transformation matches the app's BLE provisioning flow in
`APconnectPresenterImp.setDeviceMac()`.

**No authentication token is required** for temperature queries — only the
device ID.

#### Get Temperature
```
GET /V1.0/thermo/temperature?devmac=02CC445566

Response:
{
    "isonline": true,
    "time": "2026-04-17T23:49:21.010199287+08:00",
    "temperature_ch1": 21.6,
    "temperature_ch2": 0,
    "temperature_ch3": 0,
    "temperature_ch4": 0,
    "temperature_ch5": 0,
    "temperature_ch6": 0
}

Error Response:
{
    "error_code": "102",
    "error_message": "设备不存在"
}
```

- **6 temperature channels** (not 2 as originally assumed)
- `temperature_chN`: Probe N temperature in **Celsius**
- `isonline`: Whether the device is currently connected
- `time`: Server timestamp (CST/UTC+8)
- Values of `0` mean no probe connected

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

## Device-to-Cloud UDP Binary Protocol

The device sends temperature data via **UDP** to the cloud server on port **17000**.
The protocol uses a simple fixed-length binary packet format.

- Destination: `smartserver.emaxtime.cn:17000` (resolved: `47.52.241.127`)
- Alternative endpoint found in APK: `47.52.149.125:10000` (G001 devices)
- Packet interval: ~1 second (continuous while powered on)
- Packet direction: device sends, cloud echoes back with direction flag flipped

### Temperature Packet Format (18 bytes)

```
Offset  Len  Field                 Example Value
0       1    Start delimiter       0x3C ('<')
1       1    Packet type           0x54 ('T' = Temperature)
2       5    Device ID             02 CC 44 55 66 (raw bytes of cloud device ID)
7       2    Unit config           0x30 0x30 (ASCII "00" = Celsius)
9       1    Direction             0x00 = device→cloud, 0x01 = cloud→device (echo)
10      1    Temp byte count       0x04 (4 bytes = 2 channels × 2 bytes)
11      2    Temperature CH1       0x00D8 = 216 → 21.6°C (uint16 big-endian, ÷10)
13      2    Temperature CH2       0x0000 = 0.0°C (no probe connected)
15      1    Padding               0x00
16      1    Checksum              0x5A
17      1    End delimiter         0x3E ('>')
```

### Example: Captured Real Packet

Device→Cloud:
```
3C 54 02 CC 44 55 66 30 30 00 04 00 D8 00 00 00 99 3E
```
Decoded: Device `02CC445566`, CH1 = 21.6°C, CH2 = 0.0°C

Cloud→Device echo:
```
3C 54 02 CC 44 55 66 30 30 01 04 00 D8 00 00 00 9A 3E
```
Identical except: byte 9 = `0x01` (echo), byte 16 = `0x5B` (adjusted checksum)

### Temperature Encoding

Temperatures are unsigned 16-bit big-endian integers divided by 10:
- `0x00D8` = 216 → **21.6°C**
- `0x0000` = 0 → **no probe connected**
- Range: 0.0–6553.5°C (theoretical), practical: 0–500°C

### Checksum Algorithm

```
checksum = (sum(bytes[1..16]) + 0x3C) & 0xFF
```

That is: sum all bytes between the start delimiter `<` (exclusive) and the checksum
position, then add the start delimiter byte value (`0x3C`), truncate to 8 bits.

Verification for device packet:
```
sum(0x54 + 0x02 + 0xCC + 0x44 + 0x55 + 0x66 + 0x30 + 0x30 +
    0x00 + 0x04 + 0x00 + 0xD8 + 0x00 + 0x00 + 0x00)  = 0x35D
(0x35D + 0x3C) & 0xFF = 0x399 & 0xFF = 0x99 ✓
```

### Cloud Echo Behavior

The cloud server immediately echoes back the same packet with:
- Byte 9 changed from `0x00` to `0x01` (marks as cloud→device direction)
- Checksum recalculated to account for the changed byte

This echo may serve as an acknowledgment that the cloud received the data.

### Alarm Packet Format (16 bytes)

When an alarm temperature is set via the REST API (`set_alarm_temp`), the cloud
sends a **16-byte alarm command** to the device via the same UDP channel. This
sets the device's built-in alarm — the device will buzz when the temperature
exceeds the threshold.

```
Offset  Len  Field                 Example Value
0       1    Start delimiter       0x3C ('<')
1       1    Packet type           0x54 ('T' — same as temperature)
2       5    Device ID             02 CC 44 55 66 (raw bytes)
7       2    Config                0x41 0x31 (ASCII "A1" = Alarm Channel 1)
9       1    Direction             0x00 (cloud→device)
10      1    Data byte count       0x02 (2 bytes of alarm data)
11      1    Padding               0x00
12      2    Alarm temp (u16 LE)   0xEE 0x02 = 750 → 75.0°C
14      1    Checksum              0x82
15      1    End delimiter         0x3E ('>')
```

**Key differences from temperature packets:**
- **16 bytes** (not 18)
- Config bytes `"A1"` or `"A2"` (not `"00"`)
- Alarm temperature is **little-endian** u16 (temp packets use big-endian)
- Checksum covers bytes 1..14: `(sum(bytes[1..14]) + 0x3C) & 0xFF`

#### Config Byte Values

| Bytes 7-8 | ASCII | Meaning              |
|-----------|-------|----------------------|
| 30 30     | "00"  | Temperature report    |
| 41 31     | "A1"  | Alarm channel 1      |
| 41 32     | "A2"  | Alarm channel 2      |

#### Captured Examples

Set alarm to 75.0°C on channel 1:
```
3C 54 02 CC 44 55 66 41 31 00 02 00 EE 02 C1 3E
```
Decoded: `"A1"`, alarm = 0x02EE (LE) = 750 → **75.0°C**

Set alarm to 100.0°C on channel 1:
```
3C 54 02 CC 44 55 66 41 31 00 02 00 E8 03 BC 3E
```
Decoded: `"A1"`, alarm = 0x03E8 (LE) = 1000 → **100.0°C**

#### Sending Alarm Locally

The alarm can be set without the cloud by sending the packet directly to
the device's UDP source port:
```
grillsense local set-alarm --ch1 75.0
```

### UDP Proxy Architecture

A local proxy can sit between the device and cloud to extract data locally
while keeping the cloud functional:

```
Device ──UDP──► [Local Proxy :17000] ──UDP──► Cloud :17000
                     │          ▲
                     │          │ cloud echo forwarded back
                     ├──► parse temperature
                     └──► publish to MQTT → Home Assistant
```

To set this up:
1. Run the proxy: `grillsense local proxy --port 17000`
2. Redirect the device: `grillsense local configure --ip <device-ip> --ssid <ssid> -P <pass> --server <proxy-ip>`
3. The proxy forwards all traffic to the cloud (official app keeps working)
4. Optionally add `--mqtt` to publish temperatures to Home Assistant via MQTT

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
