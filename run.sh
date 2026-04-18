#!/usr/bin/with-contenv bashio

MODE=$(bashio::config 'mode')
UDP_PORT=$(bashio::config 'udp_port')
DEVICE_NAME=$(bashio::config 'device_name')

# --- Resolve MQTT credentials ---
# Priority: explicit config > HA auto-discovery
MQTT_HOST=$(bashio::config 'mqtt_host')
MQTT_PORT=$(bashio::config 'mqtt_port')
MQTT_USER=$(bashio::config 'mqtt_user')
MQTT_PASS=$(bashio::config 'mqtt_pass')

if [ -z "${MQTT_HOST}" ]; then
    if bashio::services.available "mqtt"; then
        MQTT_HOST=$(bashio::services mqtt "host")
        MQTT_PORT=$(bashio::services mqtt "port")
        MQTT_USER=$(bashio::services mqtt "username")
        MQTT_PASS=$(bashio::services mqtt "password")
        bashio::log.info "Auto-detected MQTT broker: ${MQTT_HOST}:${MQTT_PORT}"
    else
        bashio::log.warning "No MQTT service found and no mqtt_host configured — running without MQTT"
    fi
fi

MQTT_ARGS=""
if [ -n "${MQTT_HOST}" ]; then
    MQTT_ARGS="--mqtt --mqtt-host ${MQTT_HOST} --mqtt-port ${MQTT_PORT}"
    [ -n "${MQTT_USER}" ] && MQTT_ARGS="${MQTT_ARGS} --mqtt-user ${MQTT_USER}"
    [ -n "${MQTT_PASS}" ] && MQTT_ARGS="${MQTT_ARGS} --mqtt-pass ${MQTT_PASS}"
fi

# --- Launch ---
if [ "${MODE}" = "cloud" ]; then
    DEVICE_MAC=$(bashio::config 'device_mac')
    MAC_ARGS=""

    if [ -n "${DEVICE_MAC}" ]; then
        MAC_ARGS="--mac ${DEVICE_MAC}"
    else
        MAC_ARGS="--autodiscover"
        bashio::log.info "No device_mac configured — will auto-discover"
    fi

    bashio::log.info "Starting GrillSense in cloud mode..."
    exec /usr/bin/grillsense cloud monitor \
        ${MAC_ARGS} \
        ${MQTT_ARGS} \
        --device-name "${DEVICE_NAME}"
else
    bashio::log.info "Starting GrillSense in local mode on UDP port ${UDP_PORT}..."
    exec /usr/bin/grillsense local monitor \
        --port "${UDP_PORT}" \
        ${MQTT_ARGS} \
        --device-name "${DEVICE_NAME}"
fi
