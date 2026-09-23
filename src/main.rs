use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::Html,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use rumqttc::{AsyncClient, Event, Incoming, LastWill, MqttOptions, QoS};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Duration};
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;

use blueprints::{
    Blueprint, BlueprintEdge, BlueprintNode, BlueprintStore, Device, DeviceKind, NodePosition,
};
use storage::{load_store, save_store};

const BUILDING_ID: &str = "main";
const MQTT_HOST: &str = "127.0.0.1";
const MQTT_PORT: u16 = 1883;

const ROOMS: [&str; 2] = ["room-101", "room-102"];

mod blueprint_runtime;
mod blueprints;
mod storage;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MqttMessage {
    timestamp: DateTime<Utc>,
    device_id: String,
    room_id: Option<String>,
    message_type: String,
    value: Value,
    metadata: Map<String, Value>,
}

#[derive(Clone)]
struct AppState {
    mqtt: AsyncClient,
    topics: Arc<RwLock<HashMap<String, MqttMessage>>>,
    events: Arc<RwLock<Vec<MqttMessage>>>,

    blueprints: Arc<RwLock<BlueprintStore>>,
}

#[derive(Debug, Deserialize)]
struct RoomConfigInput {
    target_temperature_c: Option<f64>,
    energy_saving: Option<bool>,
    manual_mode: Option<bool>,
    blueprint_mode: Option<bool>,
}
#[derive(Debug, Deserialize)]
struct CreateDeviceInput {
    room_id: String,
    name: String,
    kind: DeviceKind,
    device_type: String,
}

async fn get_devices(State(state): State<AppState>) -> Json<Vec<Device>> {
    let store = state.blueprints.read().await;
    Json(store.devices.clone())
}

async fn create_device(
    State(state): State<AppState>,
    Json(input): Json<CreateDeviceInput>,
) -> Result<Json<Device>, (StatusCode, String)> {
    let device = Device::new(input.room_id, input.name, input.kind, input.device_type);

    let mut store = state.blueprints.write().await;
    store.devices.push(device.clone());

    save_store(&store).await.map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not save devices: {error}"),
        )
    })?;

    Ok(Json(device))
}

async fn delete_device(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let mut store = state.blueprints.write().await;

    let previous_count = store.devices.len();
    store.devices.retain(|device| device.id != id);

    if store.devices.len() == previous_count {
        return Err((StatusCode::NOT_FOUND, "Device not found".to_string()));
    }

    save_store(&store).await.map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not save devices: {error}"),
        )
    })?;

    Ok(Json(json!({ "status": "deleted" })))
}

#[derive(Debug, Deserialize)]
struct CreateBlueprintInput {
    name: String,
}

async fn get_blueprints(State(state): State<AppState>) -> Json<Vec<Blueprint>> {
    let store = state.blueprints.read().await;
    Json(store.blueprints.clone())
}

async fn create_blueprint(
    State(state): State<AppState>,
    Json(input): Json<CreateBlueprintInput>,
) -> Result<Json<Blueprint>, (StatusCode, String)> {
    let blueprint = Blueprint::empty(input.name);

    let mut store = state.blueprints.write().await;
    store.blueprints.push(blueprint.clone());

    save_store(&store).await.map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not save blueprint: {error}"),
        )
    })?;

    Ok(Json(blueprint))
}

async fn save_blueprint(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(blueprint): Json<Blueprint>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if id != blueprint.id {
        return Err((
            StatusCode::BAD_REQUEST,
            "Blueprint ID in URL does not match JSON body".to_string(),
        ));
    }

    let mut store = state.blueprints.write().await;

    let Some(existing) = store
        .blueprints
        .iter_mut()
        .find(|existing| existing.id == blueprint.id)
    else {
        return Err((StatusCode::NOT_FOUND, "Blueprint not found".to_string()));
    };

    *existing = blueprint;

    save_store(&store).await.map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not save blueprint: {error}"),
        )
    })?;

    Ok(Json(json!({ "status": "saved" })))
}

async fn delete_blueprint(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let mut store = state.blueprints.write().await;

    let previous_count = store.blueprints.len();
    store.blueprints.retain(|script| script.id != id);

    if store.blueprints.len() == previous_count {
        return Err((StatusCode::NOT_FOUND, "Blueprint not found".to_string()));
    }

    save_store(&store).await.map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not save blueprint: {error}"),
        )
    })?;

    Ok(Json(json!({ "status": "deleted" })))
}

fn default_sensor_device(room: &str, name: &str, device_type: &str) -> Device {
    Device {
        id: format!("{room}-sensor-{device_type}"),
        room_id: room.to_string(),
        name: format!("{name} ({room})"),
        kind: DeviceKind::Sensor,
        device_type: device_type.to_string(),
        mqtt_topic: format!("building/{BUILDING_ID}/room/{room}/sensor/{device_type}"),
        enabled: true,
    }
}

fn default_actuator_device(room: &str, name: &str, device_type: &str) -> Device {
    Device {
        id: format!("{room}-actuator-{device_type}"),
        room_id: room.to_string(),
        name: format!("{name} ({room})"),
        kind: DeviceKind::Actuator,
        device_type: device_type.to_string(),
        mqtt_topic: format!("building/{BUILDING_ID}/room/{room}/actuator/{device_type}/command"),
        enabled: true,
    }
}

fn edge(
    id: &str,
    source: &str,
    source_handle: &str,
    target: &str,
    target_handle: &str,
) -> BlueprintEdge {
    BlueprintEdge {
        id: id.to_string(),
        source: source.to_string(),
        source_handle: source_handle.to_string(),
        target: target.to_string(),
        target_handle: target_handle.to_string(),
    }
}

fn node(id: &str, node_type: &str, x: f64, y: f64, data: Value) -> BlueprintNode {
    BlueprintNode {
        id: id.to_string(),
        node_type: node_type.to_string(),
        position: NodePosition { x, y },
        data,
    }
}

/// Adds the simulated devices and example scripts if they are missing.
/// Existing user-created scripts and devices remain untouched.
fn ensure_default_blueprint_content(store: &mut BlueprintStore) {
    for room in ROOMS {
        let defaults = [
            default_sensor_device(room, "Temperature Sensor", "temperature"),
            default_sensor_device(room, "Humidity Sensor", "humidity"),
            default_sensor_device(room, "CO₂ Sensor", "co2"),
            default_sensor_device(room, "Occupancy Sensor", "occupancy"),
            default_actuator_device(room, "Heating", "heating"),
            default_actuator_device(room, "Ventilation", "ventilation"),
            default_actuator_device(room, "Lights", "lights"),
        ];

        for device in defaults {
            if !store
                .devices
                .iter()
                .any(|existing| existing.id == device.id)
            {
                store.devices.push(device);
            }
        }

        let heating_script_id = format!("builtin-temperature-heating-{room}");

        if !store
            .blueprints
            .iter()
            .any(|script| script.id == heating_script_id)
        {
            store.blueprints.push(Blueprint {
                id: heating_script_id,
                name: format!("{room}: Heat when temperature is below 21.5 °C"),
                enabled: true,
                nodes: vec![
                    node(
                        "temperature-sensor",
                        "sensor",
                        80.0,
                        120.0,
                        json!({
                            "device_id": format!("{room}-sensor-temperature")
                        }),
                    ),
                    node(
                        "temperature-limit",
                        "constant_number",
                        90.0,
                        310.0,
                        json!({ "value": 21.5 }),
                    ),
                    node("temperature-check", "compare_less", 390.0, 155.0, json!({})),
                    node(
                        "heating-output",
                        "actuator",
                        700.0,
                        155.0,
                        json!({
                            "device_id": format!("{room}-actuator-heating")
                        }),
                    ),
                ],
                edges: vec![
                    edge(
                        "temperature-to-check",
                        "temperature-sensor",
                        "value",
                        "temperature-check",
                        "a",
                    ),
                    edge(
                        "limit-to-check",
                        "temperature-limit",
                        "value",
                        "temperature-check",
                        "b",
                    ),
                    edge(
                        "check-to-heating",
                        "temperature-check",
                        "result",
                        "heating-output",
                        "enabled",
                    ),
                ],
            });
        }

        let lights_script_id = format!("builtin-occupancy-lights-{room}");

        if !store
            .blueprints
            .iter()
            .any(|script| script.id == lights_script_id)
        {
            store.blueprints.push(Blueprint {
                id: lights_script_id,
                name: format!("{room}: Turn lights on when occupied"),
                enabled: true,
                nodes: vec![
                    node(
                        "occupancy-sensor",
                        "sensor",
                        100.0,
                        180.0,
                        json!({
                            "device_id": format!("{room}-sensor-occupancy")
                        }),
                    ),
                    node(
                        "lights-output",
                        "actuator",
                        480.0,
                        180.0,
                        json!({
                            "device_id": format!("{room}-actuator-lights")
                        }),
                    ),
                ],
                edges: vec![edge(
                    "occupancy-to-lights",
                    "occupancy-sensor",
                    "value",
                    "lights-output",
                    "enabled",
                )],
            });
        }

        let ventilation_script_id = format!("builtin-co2-ventilation-{room}");

        if !store
            .blueprints
            .iter()
            .any(|script| script.id == ventilation_script_id)
        {
            store.blueprints.push(Blueprint {
                id: ventilation_script_id,
                name: format!("{room}: Ventilate when CO₂ is above 1000 ppm"),
                enabled: true,
                nodes: vec![
                    node(
                        "co2-sensor",
                        "sensor",
                        80.0,
                        120.0,
                        json!({
                            "device_id": format!("{room}-sensor-co2")
                        }),
                    ),
                    node(
                        "co2-limit",
                        "constant_number",
                        90.0,
                        310.0,
                        json!({ "value": 1000 }),
                    ),
                    node("co2-check", "compare_greater", 390.0, 155.0, json!({})),
                    node(
                        "ventilation-output",
                        "actuator",
                        700.0,
                        155.0,
                        json!({
                            "device_id": format!("{room}-actuator-ventilation")
                        }),
                    ),
                ],
                edges: vec![
                    edge("co2-to-check", "co2-sensor", "value", "co2-check", "a"),
                    edge("co2-limit-to-check", "co2-limit", "value", "co2-check", "b"),
                    edge(
                        "co2-check-to-ventilation",
                        "co2-check",
                        "result",
                        "ventilation-output",
                        "enabled",
                    ),
                ],
            });
        }
    }
}

#[tokio::main]
async fn main() {
    let dashboard_mqtt = create_client("dashboard-service", true);

    let mut saved_blueprints = load_store().await;

    ensure_default_blueprint_content(&mut saved_blueprints);

    if let Err(error) = save_store(&saved_blueprints).await {
        eprintln!("Could not save default Blueprint devices/scripts: {error}");
    }

    if saved_blueprints.devices.is_empty() {
        saved_blueprints = default_blueprint_store();

        if let Err(error) = save_store(&saved_blueprints).await {
            eprintln!("Could not persist default blueprint devices: {error}");
        }
    }

    let state = AppState {
        mqtt: dashboard_mqtt.0.clone(),
        topics: Arc::new(RwLock::new(HashMap::new())),
        events: Arc::new(RwLock::new(Vec::new())),
        blueprints: Arc::new(RwLock::new(saved_blueprints)),
    };

    // Poll the dashboard MQTT connection.
    tokio::spawn(run_event_loop("dashboard-service", dashboard_mqtt.1, None));

    // MQTT observer: receives all messages for dashboard state.
    tokio::spawn(dashboard_observer(state.clone()));

    // Simulated independent MQTT clients.
    for room in ROOMS {
        tokio::spawn(sensor_simulator(room.to_string()));
        tokio::spawn(actuator_simulator(room.to_string()));
        tokio::spawn(controller_service(room.to_string()));
    }

    let app = Router::new()
        .route("/", get(index))
        .route("/api/state", get(get_state))
        .route("/api/events", get(get_events))
        .route("/api/devices", get(get_devices))
        .route("/api/devices", post(create_device))
        .route("/api/devices/{id}", axum::routing::delete(delete_device))
        .route("/api/blueprints", get(get_blueprints))
        .route("/api/blueprints", post(create_blueprint))
        .route("/api/blueprints/{id}", post(save_blueprint))
        .route(
            "/api/blueprints/{id}",
            axum::routing::delete(delete_blueprint),
        )
        .route("/api/room/{room}/config", post(update_room_config))
        .route("/api/room/{room}/failure", post(simulate_failure))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let address = SocketAddr::from(([0, 0, 0, 0], 3000));

    println!("Dashboard available at http://{address}");
    println!("MQTT broker: mqtt://{MQTT_HOST}:{MQTT_PORT}");

    let listener = tokio::net::TcpListener::bind(address)
        .await
        .expect("Could not bind HTTP server");

    axum::serve(listener, app)
        .await
        .expect("HTTP server failed");
}

/// Creates an MQTT client with an LWT availability message.
fn create_client(client_id: &str, retained_will: bool) -> (AsyncClient, rumqttc::EventLoop) {
    let availability_topic = format!("building/{BUILDING_ID}/device/{client_id}/availability");

    let offline_message = MqttMessage {
        timestamp: Utc::now(),
        device_id: client_id.to_string(),
        room_id: None,
        message_type: "availability".to_string(),
        value: json!({
            "status": "offline",
            "reason": "unexpected_disconnect"
        }),
        metadata: Map::new(),
    };

    let will_payload = serde_json::to_vec(&offline_message).expect("Could not serialize LWT");

    let will = LastWill::new(
        availability_topic,
        will_payload,
        QoS::AtLeastOnce,
        retained_will,
    );

    let mut options = MqttOptions::new(client_id, MQTT_HOST, MQTT_PORT);
    options.set_keep_alive(Duration::from_secs(10));
    options.set_last_will(will);

    AsyncClient::new(options, 100)
}

async fn run_event_loop(
    client_name: &str,
    mut event_loop: rumqttc::EventLoop,
    _state: Option<AppState>,
) {
    loop {
        match event_loop.poll().await {
            Ok(_) => {}
            Err(error) => {
                eprintln!("MQTT event loop error for {client_name}: {error}");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

/// Publishes a retained online availability message.
async fn publish_online(client: &AsyncClient, device_id: &str, room_id: Option<&str>) {
    let topic = format!("building/{BUILDING_ID}/device/{device_id}/availability");

    let message = MqttMessage {
        timestamp: Utc::now(),
        device_id: device_id.to_string(),
        room_id: room_id.map(str::to_string),
        message_type: "availability".to_string(),
        value: json!({ "status": "online" }),
        metadata: Map::new(),
    };

    let _ = client
        .publish(
            topic,
            QoS::AtLeastOnce,
            true,
            serde_json::to_vec(&message).unwrap(),
        )
        .await;
}

/* -------------------------------------------------------------------------- */
/* Sensor simulator                                                           */
/* -------------------------------------------------------------------------- */

async fn sensor_simulator(room: String) {
    let device_id = format!("sensor-{room}");
    let (client, mut event_loop) = create_client(&device_id, true);

    tokio::spawn(async move {
        loop {
            if let Err(error) = event_loop.poll().await {
                eprintln!("Sensor MQTT error: {error}");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    });

    publish_online(&client, &device_id, Some(&room)).await;

    let mut sequence: u64 = 0;

    loop {
        sequence += 1;

        // Deterministic simulation values; no additional random crate required.
        let room_offset = if room == "room-101" { 0.0 } else { 2.7 };
        let phase = sequence as f64 / 8.0 + room_offset;
        let temperature = 20.5 + phase.sin() * 1.8;
        let humidity = 42.0 + phase.cos() * 7.0;
        let co2 = 650.0 + (phase.sin() + 1.0) * 300.0;
        let occupancy_offset = if room == "room-101" { 0 } else { 6 };
        let occupied = ((sequence + occupancy_offset) / 12) % 2 == 0;

        publish_measurement(
            &client,
            &room,
            &device_id,
            "temperature",
            json!({ "temperature_c": round2(temperature) }),
            sequence,
        )
        .await;

        publish_measurement(
            &client,
            &room,
            &device_id,
            "humidity",
            json!({ "humidity_percent": round2(humidity) }),
            sequence,
        )
        .await;

        publish_measurement(
            &client,
            &room,
            &device_id,
            "co2",
            json!({ "co2_ppm": round2(co2) }),
            sequence,
        )
        .await;

        publish_measurement(
            &client,
            &room,
            &device_id,
            "occupancy",
            json!({ "occupied": occupied }),
            sequence,
        )
        .await;

        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

/// QoS 0: telemetry is frequent; a lost measurement is acceptable.
async fn publish_measurement(
    client: &AsyncClient,
    room: &str,
    device_id: &str,
    sensor_type: &str,
    value: Value,
    sequence: u64,
) {
    let topic = format!("building/{BUILDING_ID}/room/{room}/sensor/{sensor_type}");

    let message = MqttMessage {
        timestamp: Utc::now(),
        device_id: device_id.to_string(),
        room_id: Some(room.to_string()),
        message_type: "measurement".to_string(),
        value,
        metadata: Map::from_iter([
            ("simulation".to_string(), json!(true)),
            ("sequence".to_string(), json!(sequence)),
        ]),
    };

    let _ = client
        .publish(
            topic,
            QoS::AtMostOnce,
            false,
            serde_json::to_vec(&message).unwrap(),
        )
        .await;
}

/* -------------------------------------------------------------------------- */
/* Controller                                                                 */
/* -------------------------------------------------------------------------- */

async fn controller_service(room: String) {
    let device_id = format!("controller-{room}");
    let (client, mut event_loop) = create_client(&device_id, true);

    let subscription = format!("building/{BUILDING_ID}/room/{room}/#");

    client
        .subscribe(subscription, QoS::AtLeastOnce)
        .await
        .expect("Controller subscription failed");

    publish_online(&client, &device_id, Some(&room)).await;

    let mut temperature = 21.0;
    let mut co2 = 700.0;
    let mut occupied = false;
    let mut target_temperature = 22.0;
    let mut energy_saving = false;
    let mut manual_mode = false;
    let mut blueprint_mode = true;

    loop {
        match event_loop.poll().await {
            Ok(Event::Incoming(Incoming::Publish(publish))) => {
                let Ok(message) = serde_json::from_slice::<MqttMessage>(&publish.payload) else {
                    continue;
                };

                let is_sensor_message = publish.topic.contains("/sensor/");
                let is_config_message = publish.topic.ends_with("/config");

                if !is_sensor_message && !is_config_message {
                    continue;
                }

                if publish.topic.ends_with("/sensor/temperature") {
                    temperature = message.value["temperature_c"]
                        .as_f64()
                        .unwrap_or(temperature);
                }

                if publish.topic.ends_with("/sensor/co2") {
                    co2 = message.value["co2_ppm"].as_f64().unwrap_or(co2);
                }

                if publish.topic.ends_with("/sensor/occupancy") {
                    occupied = message.value["occupied"].as_bool().unwrap_or(occupied);
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
                        &room,
                        "heating",
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
                        &room,
                        "ventilation",
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
                        &room,
                        "lights",
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
                            &room,
                            "alert",
                            "high_co2",
                            json!({ "co2_ppm": co2 }),
                        )
                        .await;
                    }
                }
            }
            Ok(_) => {}
            Err(error) => {
                eprintln!("Controller error for {room}: {error}");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

/// QoS 1: commands must be delivered at least once.
/// They are idempotent because they express the desired state.
async fn publish_command(
    client: &AsyncClient,
    room: &str,
    actuator: &str,
    value: Value,
    reason: &str,
) {
    let topic = format!("building/{BUILDING_ID}/room/{room}/actuator/{actuator}/command");

    let message = MqttMessage {
        timestamp: Utc::now(),
        device_id: "controller".to_string(),
        room_id: Some(room.to_string()),
        message_type: "command".to_string(),
        value,
        metadata: Map::from_iter([
            ("reason".to_string(), json!(reason)),
            (
                "correlation_id".to_string(),
                json!(format!("cmd-{}", Utc::now().timestamp_millis())),
            ),
        ]),
    };

    let _ = client
        .publish(
            topic,
            QoS::AtLeastOnce,
            false,
            serde_json::to_vec(&message).unwrap(),
        )
        .await;
}

async fn publish_event(
    client: &AsyncClient,
    room: &str,
    event_type: &str,
    reason: &str,
    value: Value,
) {
    let topic = format!("building/{BUILDING_ID}/room/{room}/event");

    let message = MqttMessage {
        timestamp: Utc::now(),
        device_id: "controller".to_string(),
        room_id: Some(room.to_string()),
        message_type: event_type.to_string(),
        value,
        metadata: Map::from_iter([("reason".to_string(), json!(reason))]),
    };

    let _ = client
        .publish(
            topic,
            QoS::AtLeastOnce,
            false,
            serde_json::to_vec(&message).unwrap(),
        )
        .await;
}

/* -------------------------------------------------------------------------- */
/* Actuator simulator                                                         */
/* -------------------------------------------------------------------------- */

async fn actuator_simulator(room: String) {
    let device_id = format!("actuator-{room}");
    let (client, mut event_loop) = create_client(&device_id, true);

    let command_topic = format!("building/{BUILDING_ID}/room/{room}/actuator/+/command");

    client
        .subscribe(command_topic, QoS::AtLeastOnce)
        .await
        .expect("Actuator subscription failed");

    publish_online(&client, &device_id, Some(&room)).await;

    loop {
        match event_loop.poll().await {
            Ok(Event::Incoming(Incoming::Publish(publish))) => {
                let Ok(command) = serde_json::from_slice::<MqttMessage>(&publish.payload) else {
                    continue;
                };

                let actuator = publish.topic.split('/').nth(6).unwrap_or("unknown");

                let enabled = command.value["enabled"].as_bool().unwrap_or(false);

                let power_percent = if enabled {
                    match actuator {
                        "heating" => 70,
                        "ventilation" => 55,
                        "lights" => 35,
                        _ => 0,
                    }
                } else {
                    0
                };

                let state_topic =
                    format!("building/{BUILDING_ID}/room/{room}/actuator/{actuator}/state");

                let state_message = MqttMessage {
                    timestamp: Utc::now(),
                    device_id: device_id.clone(),
                    room_id: Some(room.clone()),
                    message_type: "state".to_string(),
                    value: json!({
                        "enabled": enabled,
                        "power_percent": power_percent,
                        "mode": command.value["mode"].as_str().unwrap_or("automatic")
                    }),
                    metadata: Map::from_iter([(
                        "correlation_id".to_string(),
                        command
                            .metadata
                            .get("correlation_id")
                            .cloned()
                            .unwrap_or(Value::Null),
                    )]),
                };

                let _ = client
                    .publish(
                        state_topic,
                        QoS::AtLeastOnce,
                        true,
                        serde_json::to_vec(&state_message).unwrap(),
                    )
                    .await;
            }
            Ok(_) => {}
            Err(error) => {
                eprintln!("Actuator error for {room}: {error}");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

async fn dashboard_observer(state: AppState) {
    let observer_id = "dashboard-observer";
    let (client, mut event_loop) = create_client(observer_id, true);

    client
        .subscribe(format!("building/{BUILDING_ID}/#"), QoS::AtLeastOnce)
        .await
        .expect("Dashboard MQTT subscription failed");

    publish_online(&client, observer_id, None).await;

    loop {
        match event_loop.poll().await {
            Ok(Event::Incoming(Incoming::Publish(publish))) => {
                if let Ok(message) = serde_json::from_slice::<MqttMessage>(&publish.payload) {
                    state
                        .topics
                        .write()
                        .await
                        .insert(publish.topic.clone(), message.clone());

                    if publish.topic.contains("/sensor/") {
                        let topic_cache = state.topics.read().await.clone();
                        let store = state.blueprints.read().await.clone();

                        blueprint_runtime::execute_blueprints(&state.mqtt, &store, &topic_cache)
                            .await;
                    }

                    if publish.topic.ends_with("/event") || publish.topic.ends_with("/availability")
                    {
                        let mut events = state.events.write().await;
                        events.push(message);

                        if events.len() > 100 {
                            events.remove(0);
                        }
                    }
                }
            }
            Ok(_) => {}
            Err(error) => {
                eprintln!("Dashboard observer error: {error}");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

async fn get_state(State(state): State<AppState>) -> Json<Value> {
    let topics = state.topics.read().await;

    let result: HashMap<String, MqttMessage> = topics.clone();

    Json(json!({
        "building_id": BUILDING_ID,
        "timestamp": Utc::now(),
        "topics": result
    }))
}

async fn get_events(State(state): State<AppState>) -> Json<Vec<MqttMessage>> {
    Json(state.events.read().await.clone())
}

async fn update_room_config(
    State(state): State<AppState>,
    Path(room): Path<String>,
    Json(input): Json<RoomConfigInput>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if !ROOMS.contains(&room.as_str()) {
        return Err((StatusCode::NOT_FOUND, format!("Unknown room: {room}")));
    }

    let topic = format!("building/{BUILDING_ID}/room/{room}/config");

    let message = MqttMessage {
        timestamp: Utc::now(),
        device_id: "dashboard".to_string(),
        room_id: Some(room.clone()),
        message_type: "config".to_string(),
        value: json!({
            "target_temperature_c": input.target_temperature_c,
            "energy_saving": input.energy_saving,
            "manual_mode": input.manual_mode,
            "blueprint_mode": input.blueprint_mode
        }),
        metadata: Map::from_iter([("source".to_string(), json!("dashboard"))]),
    };

    state
        .mqtt
        .publish(
            topic,
            QoS::ExactlyOnce,
            false,
            serde_json::to_vec(&message).unwrap(),
        )
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("MQTT publish failed: {error}"),
            )
        })?;

    Ok(Json(json!({
        "status": "configuration published",
        "qos": 2,
        "room": room
    })))
}

/// Simulates a device outage for demonstration purposes.
/// A real LWT event appears if an actuator process disconnects unexpectedly.
async fn simulate_failure(
    State(state): State<AppState>,
    Path(room): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let device_id = format!("actuator-{room}");
    let topic = format!("building/{BUILDING_ID}/device/{device_id}/availability");

    let message = MqttMessage {
        timestamp: Utc::now(),
        device_id,
        room_id: Some(room.clone()),
        message_type: "availability".to_string(),
        value: json!({
            "status": "offline",
            "reason": "simulated_failure"
        }),
        metadata: Map::from_iter([("simulation".to_string(), json!(true))]),
    };

    state
        .mqtt
        .publish(
            topic,
            QoS::AtLeastOnce,
            true,
            serde_json::to_vec(&message).unwrap(),
        )
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("MQTT publish failed: {error}"),
            )
        })?;

    Ok(Json(json!({
        "status": "simulated actuator failure published",
        "room": room
    })))
}

fn round2(number: f64) -> f64 {
    (number * 100.0).round() / 100.0
}

const DASHBOARD_HTML: &str = include_str!("../web/index.html");

async fn index() -> Html<&'static str> {
    Html(DASHBOARD_HTML)
}

fn default_blueprint_store() -> BlueprintStore {
    let mut devices = Vec::new();

    for room in ROOMS {
        for (name, device_type) in [
            ("Temperature Sensor", "temperature"),
            ("Humidity Sensor", "humidity"),
            ("CO₂ Sensor", "co2"),
            ("Occupancy Sensor", "occupancy"),
        ] {
            devices.push(Device {
                id: format!("{room}-sensor-{device_type}"),
                room_id: room.to_string(),
                name: format!("{name} ({room})"),
                kind: DeviceKind::Sensor,
                device_type: device_type.to_string(),
                mqtt_topic: format!("building/{BUILDING_ID}/room/{room}/sensor/{device_type}"),
                enabled: true,
            });
        }

        for (name, device_type) in [
            ("Heating", "heating"),
            ("Ventilation", "ventilation"),
            ("Lights", "lights"),
        ] {
            devices.push(Device {
                id: format!("{room}-actuator-{device_type}"),
                room_id: room.to_string(),
                name: format!("{name} ({room})"),
                kind: DeviceKind::Actuator,
                device_type: device_type.to_string(),
                mqtt_topic: format!(
                    "building/{BUILDING_ID}/room/{room}/actuator/{device_type}/command"
                ),
                enabled: true,
            });
        }
    }

    BlueprintStore {
        devices,
        blueprints: Vec::new(),
    }
}
