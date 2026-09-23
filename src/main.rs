use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::Html,
    routing::{delete, get, post},
};
use chrono::{DateTime, Utc};
use rumqttc::{AsyncClient, Event, Incoming, LastWill, MqttOptions, QoS};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Duration};
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;

use blueprints::{Blueprint, BlueprintStore, Device, DeviceKind};
use storage::{load_store, save_store};

const BUILDING_ID: &str = "main";
const MQTT_HOST: &str = "127.0.0.1";
const MQTT_PORT: u16 = 1883;

mod blueprint_runtime;
mod blueprints;
mod building_api;
mod simulators;
mod storage;
mod topics;

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

    /// Room id -> player ids currently walking through that room.
    presence: simulators::PresenceMap,

    /// Room id -> running simulator tasks.
    simulators: simulators::SimulatorRegistry,
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
    let mut store = state.blueprints.write().await;

    let floor_id = store.building.floor_id_for_room(&input.room_id).ok_or((
        StatusCode::NOT_FOUND,
        format!("Unknown room: {}", input.room_id),
    ))?;

    let device = Device::for_room(
        &floor_id,
        &input.room_id,
        input.name,
        input.kind,
        input.device_type,
    );

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

#[tokio::main]
async fn main() {
    let address = SocketAddr::from(([0, 0, 0, 0], 3000));

    // Bind before anything else starts, so a port conflict fails instantly
    // instead of after every MQTT client and simulator has been spawned.
    let listener = match tokio::net::TcpListener::bind(address).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("Could not bind http://{address}: {error}");
            eprintln!(
                "Port 3000 is already in use — another instance of the simulator is probably still running."
            );
            std::process::exit(1);
        }
    };

    let dashboard_mqtt = create_client("dashboard-service", true);

    let mut store = load_store().await;

    // Older data files gain the Building -> Floor -> Room tree here,
    // and every device topic is rebuilt from that tree.
    blueprints::migrate_store(&mut store);
    blueprints::ensure_default_content(&mut store);

    if let Err(error) = save_store(&store).await {
        eprintln!("Could not save default Blueprint devices/scripts: {error}");
    }

    purge_legacy_retained(&dashboard_mqtt.0, &store).await;

    let state = AppState {
        mqtt: dashboard_mqtt.0.clone(),
        topics: Arc::new(RwLock::new(HashMap::new())),
        events: Arc::new(RwLock::new(Vec::new())),
        blueprints: Arc::new(RwLock::new(store)),
        presence: Arc::new(RwLock::new(HashMap::new())),
        simulators: Arc::new(RwLock::new(HashMap::new())),
    };

    // Poll the dashboard MQTT connection.
    tokio::spawn(run_event_loop("dashboard-service", dashboard_mqtt.1, None));

    // MQTT observer: receives all messages for dashboard state.
    tokio::spawn(dashboard_observer(state.clone()));

    // Simulated independent MQTT clients for every room in the building.
    let startup_rooms = {
        let store = state.blueprints.read().await;

        store
            .building
            .rooms()
            .into_iter()
            .map(|(floor, room)| (floor.id.clone(), room.id.clone()))
            .collect::<Vec<_>>()
    };

    for (floor_id, room_id) in startup_rooms {
        simulators::spawn_room(&state, floor_id, room_id).await;
    }

    let app = Router::new()
        .route("/", get(index))
        .route("/api/state", get(get_state))
        .route("/api/events", get(get_events))
        .route("/api/devices", get(get_devices))
        .route("/api/devices", post(create_device))
        .route("/api/devices/{id}", delete(delete_device))
        .route("/api/blueprints", get(get_blueprints))
        .route("/api/blueprints", post(create_blueprint))
        .route("/api/blueprints/{id}", post(save_blueprint))
        .route("/api/blueprints/{id}", delete(delete_blueprint))
        .route("/api/room/{room}/config", post(update_room_config))
        .route("/api/room/{room}/failure", post(simulate_failure))
        .merge(building_api::router())
        .layer(CorsLayer::permissive())
        .with_state(state);

    println!("Dashboard available at http://{address}");
    println!("MQTT broker: mqtt://{MQTT_HOST}:{MQTT_PORT}");

    axum::serve(listener, app)
        .await
        .expect("HTTP server failed");
}

/// Creates an MQTT client with an LWT availability message.
fn create_client(client_id: &str, retained_will: bool) -> (AsyncClient, rumqttc::EventLoop) {
    let availability_topic = topics::device_availability(client_id);

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
/// Older versions stored retained actuator states on
/// `building/{id}/room/{room}/...` (no floor level). Publishing an empty
/// retained payload clears them on the broker.
async fn purge_legacy_retained(client: &AsyncClient, store: &BlueprintStore) {
    let actuator_types = [
        "heating",
        "ventilation",
        "lights",
        "door_lock",
        "garage_door",
    ];

    for (_, room) in store.building.rooms() {
        for actuator in actuator_types {
            let topic = format!(
                "building/{BUILDING_ID}/room/{}/actuator/{actuator}/state",
                room.id
            );

            let _ = client
                .publish(topic, QoS::AtMostOnce, true, Vec::new())
                .await;
        }
    }
}

async fn publish_online(client: &AsyncClient, device_id: &str, room_id: Option<&str>) {
    let topic = topics::device_availability(device_id);

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

/// Retained leftovers from deleted rooms or from the pre-floor topic scheme
/// must never re-enter `/api/state` (they reappear after a restart because
/// the broker redelivers retained messages and fires LWTs on room deletion).
async fn is_stale(state: &AppState, topic: &str, message: &MqttMessage) -> bool {
    // Simulator availability messages only carry the room inside the device id.
    let room_id = message.room_id.clone().or_else(|| {
        ["sensor-", "actuator-", "controller-", "energy-meter-"]
            .iter()
            .find_map(|prefix| message.device_id.strip_prefix(prefix))
            .map(str::to_string)
    });

    let Some(room_id) = room_id else {
        return false;
    };

    let store = state.blueprints.read().await;

    let Some(floor_id) = store.building.floor_id_for_room(&room_id) else {
        // The room was deleted.
        return true;
    };

    if !topic.contains("/room/") {
        return false;
    }

    // Leftover of the topic scheme without the floor level.
    !topic.starts_with(&format!(
        "building/{BUILDING_ID}/floor/{floor_id}/room/{room_id}/"
    ))
}

async fn dashboard_observer(state: AppState) {
    let observer_id = "dashboard-observer";
    let (client, mut event_loop) = create_client(observer_id, true);

    client
        .subscribe(topics::building_filter(), QoS::AtLeastOnce)
        .await
        .expect("Dashboard MQTT subscription failed");

    publish_online(&client, observer_id, None).await;

    loop {
        match event_loop.poll().await {
            Ok(Event::Incoming(Incoming::Publish(publish))) => {
                if let Ok(message) = serde_json::from_slice::<MqttMessage>(&publish.payload) {
                    if is_stale(&state, &publish.topic, &message).await {
                        continue;
                    }

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
    let floor_id = {
        let store = state.blueprints.read().await;

        store
            .building
            .floor_id_for_room(&room)
            .ok_or((StatusCode::NOT_FOUND, format!("Unknown room: {room}")))?
    };

    let topic = topics::room_config(&floor_id, &room);

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
    let topic = topics::device_availability(&device_id);

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
