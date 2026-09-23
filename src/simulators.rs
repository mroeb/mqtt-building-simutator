//! Per-room simulation tasks: sensors, actuators, controller and energy meter.
//!
//! Every room owns four tasks. They are started with [`spawn_room`] and
//! stopped with [`stop_room`], so rooms can be added and removed at runtime.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rumqttc::{AsyncClient, Event, Incoming, QoS};
use serde_json::{Map, Value, json};
use tokio::sync::{RwLock, watch};

use crate::{AppState, MqttMessage, create_client, publish_online, round2, topics};

/// Room id -> shutdown signal of that room's tasks.
pub type SimulatorRegistry = Arc<RwLock<HashMap<String, watch::Sender<bool>>>>;

/// Room id -> ids of the players currently walking inside that room.
pub type PresenceMap = Arc<RwLock<HashMap<String, HashSet<String>>>>;

/* -------------------------------------------------------------------------- */
/* Task registry                                                              */
/* -------------------------------------------------------------------------- */

/// Starts all simulator tasks for one room. Does nothing if they already run.
pub async fn spawn_room(state: &AppState, floor_id: String, room_id: String) {
    let mut registry = state.simulators.write().await;

    if registry.contains_key(&room_id) {
        return;
    }

    let (shutdown, receiver) = watch::channel(false);
    registry.insert(room_id.clone(), shutdown);
    drop(registry);

    tokio::spawn(sensor_simulator(
        state.presence.clone(),
        floor_id.clone(),
        room_id.clone(),
        receiver.clone(),
    ));
    tokio::spawn(actuator_simulator(
        floor_id.clone(),
        room_id.clone(),
        receiver.clone(),
    ));
    tokio::spawn(controller_service(
        floor_id.clone(),
        room_id.clone(),
        receiver.clone(),
    ));
    tokio::spawn(energy_meter(floor_id, room_id, receiver));
}

/// Stops all simulator tasks of one room.
pub async fn stop_room(state: &AppState, room_id: &str) {
    let shutdown = state.simulators.write().await.remove(room_id);

    if let Some(shutdown) = shutdown {
        let _ = shutdown.send(true);
    }
}

/* -------------------------------------------------------------------------- */
/* Shared helpers                                                             */
/* -------------------------------------------------------------------------- */

fn rated_watts(actuator: &str) -> f64 {
    match actuator {
        "heating" => 2000.0,
        "ventilation" => 800.0,
        "lights" => 250.0,
        "garage_door" => 700.0,
        "door_lock" => 100.0,
        _ => 200.0,
    }
}

/// Share of rated power an enabled actuator runs at.
fn power_percent_when_on(actuator: &str) -> u8 {
    match actuator {
        "heating" => 70,
        "ventilation" => 55,
        "lights" => 35,
        _ => 50,
    }
}

/// QoS 0: telemetry is frequent; a lost measurement is acceptable.
async fn publish_measurement(
    client: &AsyncClient,
    floor_id: &str,
    room_id: &str,
    device_id: &str,
    sensor_type: &str,
    value: Value,
    sequence: u64,
) {
    let message = MqttMessage {
        timestamp: chrono::Utc::now(),
        device_id: device_id.to_string(),
        room_id: Some(room_id.to_string()),
        message_type: "measurement".to_string(),
        value,
        metadata: Map::from_iter([
            ("simulation".to_string(), json!(true)),
            ("sequence".to_string(), json!(sequence)),
        ]),
    };

    let _ = client
        .publish(
            topics::sensor(floor_id, room_id, sensor_type),
            QoS::AtMostOnce,
            false,
            serde_json::to_vec(&message).unwrap(),
        )
        .await;
}

/// QoS 1: commands must be delivered at least once.
/// They are idempotent because they express the desired state.
async fn publish_command(
    client: &AsyncClient,
    floor_id: &str,
    room_id: &str,
    actuator: &str,
    device_id: &str,
    value: Value,
    reason: &str,
) {
    let message = MqttMessage {
        timestamp: chrono::Utc::now(),
        device_id: device_id.to_string(),
        room_id: Some(room_id.to_string()),
        message_type: "command".to_string(),
        value,
        metadata: Map::from_iter([
            ("reason".to_string(), json!(reason)),
            (
                "correlation_id".to_string(),
                json!(format!("cmd-{}", chrono::Utc::now().timestamp_millis())),
            ),
        ]),
    };

    let _ = client
        .publish(
            topics::actuator_command(floor_id, room_id, actuator),
            QoS::AtLeastOnce,
            false,
            serde_json::to_vec(&message).unwrap(),
        )
        .await;
}

pub async fn publish_event(
    client: &AsyncClient,
    floor_id: &str,
    room_id: &str,
    event_type: &str,
    reason: &str,
    value: Value,
) {
    let message = MqttMessage {
        timestamp: chrono::Utc::now(),
        device_id: "controller".to_string(),
        room_id: Some(room_id.to_string()),
        message_type: event_type.to_string(),
        value,
        metadata: Map::from_iter([("reason".to_string(), json!(reason))]),
    };

    let _ = client
        .publish(
            topics::room_event(floor_id, room_id),
            QoS::AtLeastOnce,
            false,
            serde_json::to_vec(&message).unwrap(),
        )
        .await;
}

/* -------------------------------------------------------------------------- */
/* Sensor simulator                                                           */
/* -------------------------------------------------------------------------- */

async fn sensor_simulator(
    presence: PresenceMap,
    floor_id: String,
    room_id: String,
    mut shutdown: watch::Receiver<bool>,
) {
    let device_id = format!("sensor-{room_id}");
    let (client, mut event_loop) = create_client(&device_id, true);

    let lights_topic = topics::actuator_state(&floor_id, &room_id, "lights");

    if let Err(error) = client
        .subscribe(lights_topic.clone(), QoS::AtLeastOnce)
        .await
    {
        eprintln!("Sensor {room_id} subscription failed: {error}");
    }

    publish_online(&client, &device_id, Some(&room_id)).await;

    // Deterministic simulation values; no additional random crate required.
    let seed: u32 = room_id.bytes().map(u32::from).sum();
    let phase_offset = (seed % 7) as f64;
    let occupancy_offset = u64::from(seed % 5);

    let mut lights_on = false;
    let mut sequence: u64 = 0;
    let mut interval = tokio::time::interval(Duration::from_secs(3));

    loop {
        tokio::select! {
            _ = shutdown.changed() => break,

            result = event_loop.poll() => match result {
                Ok(Event::Incoming(Incoming::Publish(publish))) => {
                    if publish.topic == lights_topic {
                        lights_on = serde_json::from_slice::<MqttMessage>(&publish.payload)
                            .map(|message| message.value["enabled"].as_bool().unwrap_or(false))
                            .unwrap_or(lights_on);
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    eprintln!("Sensor MQTT error for {room_id}: {error}");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            },

            _ = interval.tick() => {
                sequence += 1;

                let players_present = presence
                    .read()
                    .await
                    .get(&room_id)
                    .is_some_and(|players| !players.is_empty());

                let phase = sequence as f64 / 8.0 + phase_offset;

                let temperature = 20.5 + phase.sin() * 1.8;
                let humidity = 42.0 + phase.cos() * 7.0;
                let co2 = 650.0 + (phase.sin() + 1.0) * 300.0;

                let simulated_occupied = ((sequence + occupancy_offset) / 12).is_multiple_of(2);
                let occupied = simulated_occupied || players_present;
                let motion = players_present || simulated_occupied;

                let illuminance = if lights_on {
                    480.0 + phase.sin() * 25.0
                } else {
                    65.0 + phase.cos() * 15.0
                };

                for (sensor_type, value) in [
                    ("temperature", json!({ "temperature_c": round2(temperature) })),
                    ("humidity", json!({ "humidity_percent": round2(humidity) })),
                    ("co2", json!({ "co2_ppm": round2(co2) })),
                    ("occupancy", json!({ "occupied": occupied })),
                    ("motion", json!({ "motion_detected": motion })),
                    ("illuminance", json!({ "illuminance_lux": round2(illuminance) })),
                ] {
                    // The energy meter owns its own topic.
                    publish_measurement(
                        &client,
                        &floor_id,
                        &room_id,
                        &device_id,
                        sensor_type,
                        value,
                        sequence,
                    )
                    .await;
                }
            }
        }
    }
}

/* -------------------------------------------------------------------------- */
/* Actuator simulator                                                         */
/* -------------------------------------------------------------------------- */

#[derive(Debug, Clone)]
struct ActuatorState {
    enabled: bool,
    power_percent: u8,
    mode: String,
    position: f64,
    moving: bool,
}

impl Default for ActuatorState {
    fn default() -> Self {
        Self {
            enabled: false,
            power_percent: 0,
            mode: "automatic".to_string(),
            position: 0.0,
            moving: false,
        }
    }
}

fn state_value(actuator: &str, state: &ActuatorState) -> Value {
    match actuator {
        "garage_door" => json!({
            "enabled": state.position > 0.5,
            "position_percent": state.position.round(),
            "moving": state.moving,
            "power_percent": state.power_percent,
            "mode": state.mode
        }),
        "door_lock" => json!({
            "enabled": state.enabled,
            "locked": state.enabled,
            "power_percent": state.power_percent,
            "mode": state.mode
        }),
        _ => json!({
            "enabled": state.enabled,
            "power_percent": state.power_percent,
            "mode": state.mode
        }),
    }
}

async fn publish_state(
    client: &AsyncClient,
    floor_id: &str,
    room_id: &str,
    device_id: &str,
    actuator: &str,
    state: &ActuatorState,
    correlation_id: Value,
) {
    let message = MqttMessage {
        timestamp: chrono::Utc::now(),
        device_id: device_id.to_string(),
        room_id: Some(room_id.to_string()),
        message_type: "state".to_string(),
        value: state_value(actuator, state),
        metadata: Map::from_iter([("correlation_id".to_string(), correlation_id)]),
    };

    let _ = client
        .publish(
            topics::actuator_state(floor_id, room_id, actuator),
            QoS::AtLeastOnce,
            true,
            serde_json::to_vec(&message).unwrap(),
        )
        .await;
}

async fn actuator_simulator(
    floor_id: String,
    room_id: String,
    mut shutdown: watch::Receiver<bool>,
) {
    let device_id = format!("actuator-{room_id}");
    let (client, mut event_loop) = create_client(&device_id, true);

    if let Err(error) = client
        .subscribe(
            topics::actuator_command_filter(&floor_id, &room_id),
            QoS::AtLeastOnce,
        )
        .await
    {
        eprintln!("Actuator {room_id} subscription failed: {error}");
    }

    publish_online(&client, &device_id, Some(&room_id)).await;

    let mut states: HashMap<String, ActuatorState> = HashMap::new();
    // Garage doors need a tick loop so they animate towards their target.
    let mut interval = tokio::time::interval(Duration::from_millis(500));

    loop {
        tokio::select! {
            _ = shutdown.changed() => break,

            result = event_loop.poll() => match result {
                Ok(Event::Incoming(Incoming::Publish(publish))) => {
                    if let Ok(command) = serde_json::from_slice::<MqttMessage>(&publish.payload)
                        && let Some(actuator) = topics::actuator_from_topic(&publish.topic)
                    {
                        let enabled = command.value["enabled"].as_bool().unwrap_or(false);
                        let mode = command.value["mode"]
                            .as_str()
                            .unwrap_or("manual")
                            .to_string();
                        let correlation_id = command
                            .metadata
                            .get("correlation_id")
                            .cloned()
                            .unwrap_or(Value::Null);

                        let state = states.entry(actuator.clone()).or_default();
                        state.mode = mode;

                        match actuator.as_str() {
                            "door_lock" => {
                                state.enabled = enabled;
                                state.power_percent = if enabled { 10 } else { 0 };
                            }
                            "garage_door" => {
                                state.enabled = enabled;
                                let target = if enabled { 100.0 } else { 0.0 };
                                state.moving = (state.position - target).abs() > 0.5;
                                state.power_percent = if state.moving { 100 } else { 0 };
                            }
                            _ => {
                                state.enabled = enabled;
                                state.power_percent =
                                    if enabled { power_percent_when_on(&actuator) } else { 0 };
                            }
                        }

                        publish_state(
                            &client,
                            &floor_id,
                            &room_id,
                            &device_id,
                            &actuator,
                            state,
                            correlation_id,
                        )
                        .await;
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    eprintln!("Actuator error for {room_id}: {error}");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            },

            _ = interval.tick() => {
                let mut moving: Vec<String> = Vec::new();

                for (actuator, state) in states.iter_mut() {
                    if actuator != "garage_door" || !state.moving {
                        continue;
                    }

                    let target = if state.enabled { 100.0 } else { 0.0 };
                    let step = 10.0_f64.copysign(target - state.position);

                    state.position = (state.position + step).clamp(0.0, 100.0);

                    if (state.position - target).abs() < 0.5 {
                        state.position = target;
                        state.moving = false;
                    }

                    state.power_percent = if state.moving { 100 } else { 0 };
                    moving.push(actuator.clone());
                }

                for actuator in moving {
                    let state = states.get(&actuator).cloned().unwrap_or_default();

                    publish_state(
                        &client,
                        &floor_id,
                        &room_id,
                        &device_id,
                        &actuator,
                        &state,
                        Value::Null,
                    )
                    .await;
                }
            }
        }
    }
}

/* -------------------------------------------------------------------------- */
/* Controller (automatic mode)                                                */
/* -------------------------------------------------------------------------- */

async fn controller_service(
    floor_id: String,
    room_id: String,
    mut shutdown: watch::Receiver<bool>,
) {
    let device_id = format!("controller-{room_id}");
    let (client, mut event_loop) = create_client(&device_id, true);

    if let Err(error) = client
        .subscribe(topics::room_filter(&floor_id, &room_id), QoS::AtLeastOnce)
        .await
    {
        eprintln!("Controller {room_id} subscription failed: {error}");
    }

    publish_online(&client, &device_id, Some(&room_id)).await;

    let mut temperature = 21.0;
    let mut co2 = 700.0;
    let mut occupied = false;
    let mut target_temperature = 22.0;
    let mut energy_saving = false;
    let mut manual_mode = false;
    let mut blueprint_mode = true;

    loop {
        tokio::select! {
            _ = shutdown.changed() => break,

            result = event_loop.poll() => match result {
                Ok(Event::Incoming(Incoming::Publish(publish))) => {
                    if let Ok(message) = serde_json::from_slice::<MqttMessage>(&publish.payload) {
                        let is_sensor_message = publish.topic.contains("/sensor/");
                        let is_config_message = publish.topic.ends_with("/config");

                        if is_sensor_message || is_config_message {
                            if publish.topic.ends_with("/sensor/temperature") {
                                temperature = message.value["temperature_c"]
                                    .as_f64()
                                    .unwrap_or(temperature);
                            }

                            if publish.topic.ends_with("/sensor/co2") {
                                co2 = message.value["co2_ppm"].as_f64().unwrap_or(co2);
                            }

                            if publish.topic.ends_with("/sensor/occupancy") {
                                occupied = message.value["occupied"]
                                    .as_bool()
                                    .unwrap_or(occupied);
                            }

                            // QoS 2 configuration messages.
                            if publish.topic.ends_with("/config") {
                                if let Some(value) = message.value["target_temperature_c"].as_f64() {
                                    target_temperature = value;
                                }

                                if let Some(value) = message.value["energy_saving"].as_bool() {
                                    energy_saving = value;
                                }

                                if let Some(value) = message.value["manual_mode"].as_bool() {
                                    manual_mode = value;
                                }

                                if let Some(value) = message.value["blueprint_mode"].as_bool() {
                                    blueprint_mode = value;
                                }
                            }

                            if !manual_mode && !blueprint_mode {
                                let effective_target = if energy_saving {
                                    target_temperature - 1.0
                                } else {
                                    target_temperature
                                };

                                // Heating hysteresis: ON below target - 0.5; OFF above target + 0.5.
                                let heating_enabled = occupied && temperature < effective_target - 0.5;
                                let ventilation_enabled = co2 > 1000.0;
                                let lights_enabled = occupied;

                                publish_command(
                                    &client,
                                    &floor_id,
                                    &room_id,
                                    "heating",
                                    "controller",
                                    json!({
                                        "enabled": heating_enabled,
                                        "mode": if energy_saving { "eco" } else { "automatic" },
                                        "target_temperature_c": effective_target
                                    }),
                                    if heating_enabled {
                                        "temperature_below_target"
                                    } else {
                                        "target_reached_or_room_unoccupied"
                                    },
                                )
                                .await;

                                publish_command(
                                    &client,
                                    &floor_id,
                                    &room_id,
                                    "ventilation",
                                    "controller",
                                    json!({
                                        "enabled": ventilation_enabled,
                                        "mode": "automatic"
                                    }),
                                    if ventilation_enabled {
                                        "co2_above_1000ppm"
                                    } else {
                                        "co2_normal"
                                    },
                                )
                                .await;

                                publish_command(
                                    &client,
                                    &floor_id,
                                    &room_id,
                                    "lights",
                                    "controller",
                                    json!({
                                        "enabled": lights_enabled,
                                        "mode": "automatic"
                                    }),
                                    if lights_enabled {
                                        "occupancy_detected"
                                    } else {
                                        "room_unoccupied"
                                    },
                                )
                                .await;

                                if co2 > 1200.0 {
                                    publish_event(
                                        &client,
                                        &floor_id,
                                        &room_id,
                                        "alert",
                                        "high_co2",
                                        json!({ "co2_ppm": co2 }),
                                    )
                                    .await;
                                }
                            }
                        }
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    eprintln!("Controller error for {room_id}: {error}");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        }
    }
}

/* -------------------------------------------------------------------------- */
/* Energy meter                                                               */
/* -------------------------------------------------------------------------- */

/// Integrates the power of every actuator state into watt and kilowatt-hour
/// readings and publishes them on the room energy sensor topic.
async fn energy_meter(floor_id: String, room_id: String, mut shutdown: watch::Receiver<bool>) {
    let device_id = format!("energy-meter-{room_id}");
    let (client, mut event_loop) = create_client(&device_id, true);

    if let Err(error) = client
        .subscribe(
            topics::actuator_state_filter(&floor_id, &room_id),
            QoS::AtLeastOnce,
        )
        .await
    {
        eprintln!("Energy meter {room_id} subscription failed: {error}");
    }

    publish_online(&client, &device_id, Some(&room_id)).await;

    let mut powers: HashMap<String, f64> = HashMap::new();
    let mut energy_kwh = 0.0;
    let mut last_tick = Instant::now();
    let mut ticks: u64 = 0;
    let mut interval = tokio::time::interval(Duration::from_secs(1));

    loop {
        tokio::select! {
            _ = shutdown.changed() => break,

            result = event_loop.poll() => match result {
                Ok(Event::Incoming(Incoming::Publish(publish))) => {
                    if let (Some(actuator), Ok(message)) = (
                        topics::actuator_from_topic(&publish.topic),
                        serde_json::from_slice::<MqttMessage>(&publish.payload),
                    ) {
                        let percent = message.value["power_percent"].as_f64().unwrap_or(0.0);
                        powers.insert(actuator.clone(), rated_watts(&actuator) * percent / 100.0);
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    eprintln!("Energy meter error for {room_id}: {error}");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            },

            _ = interval.tick() => {
                let now = Instant::now();
                let elapsed_hours = now.duration_since(last_tick).as_secs_f64() / 3600.0;
                last_tick = now;

                let power_w: f64 = powers.values().sum();
                energy_kwh += power_w / 1000.0 * elapsed_hours;
                ticks += 1;

                if ticks.is_multiple_of(2) {
                    let message = MqttMessage {
                        timestamp: chrono::Utc::now(),
                        device_id: device_id.clone(),
                        room_id: Some(room_id.clone()),
                        message_type: "measurement".to_string(),
                        value: json!({
                            "power_w": round2(power_w),
                            "energy_kwh": (energy_kwh * 10000.0).round() / 10000.0
                        }),
                        metadata: Map::from_iter([
                            ("simulation".to_string(), json!(true)),
                            ("sequence".to_string(), json!(ticks)),
                        ]),
                    };

                    let _ = client
                        .publish(
                            topics::sensor(&floor_id, &room_id, "energy"),
                            QoS::AtMostOnce,
                            false,
                            serde_json::to_vec(&message).unwrap(),
                        )
                        .await;
                }
            }
        }
    }
}

/* -------------------------------------------------------------------------- */
/* Game presence                                                              */
/* -------------------------------------------------------------------------- */

/// Immediately reports a player entering a room so occupancy and motion
/// sensors react without waiting for the next simulation tick.
pub async fn publish_presence_entered(
    client: &AsyncClient,
    floor_id: &str,
    room_id: &str,
    players: usize,
) {
    for (sensor_type, field, value) in [
        ("occupancy", "occupied", json!(true)),
        ("motion", "motion_detected", json!(true)),
    ] {
        let message = MqttMessage {
            timestamp: chrono::Utc::now(),
            device_id: "presence-simulator".to_string(),
            room_id: Some(room_id.to_string()),
            message_type: "measurement".to_string(),
            value: json!({ field: value }),
            metadata: Map::from_iter([
                ("simulation".to_string(), json!(true)),
                ("players".to_string(), json!(players)),
                ("sequence".to_string(), json!(players)),
            ]),
        };

        let _ = client
            .publish(
                topics::sensor(floor_id, room_id, sensor_type),
                QoS::AtMostOnce,
                false,
                serde_json::to_vec(&message).unwrap(),
            )
            .await;
    }
}
