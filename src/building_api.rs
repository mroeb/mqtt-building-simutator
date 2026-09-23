//! HTTP endpoints for the Building -> Floor -> Room structure,
//! player presence and manual actuator control from the 3D walk mode.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::blueprints::{self, BlueprintStore, DeviceKind, Room, RoomKind};
use crate::simulators;
use crate::{AppState, MqttMessage, topics};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/building", get(get_building))
        .route("/api/floors", post(create_floor))
        .route("/api/floors/{id}", delete(delete_floor))
        .route("/api/floors/{id}/rooms", post(create_room))
        .route("/api/rooms/{id}", patch(update_room).delete(delete_room))
        .route("/api/presence", post(set_presence))
        .route(
            "/api/room/{room}/actuator/{actuator}",
            post(control_actuator),
        )
}

type ApiError = (StatusCode, String);

fn bad_request(message: impl Into<String>) -> ApiError {
    (StatusCode::BAD_REQUEST, message.into())
}

fn not_found(message: impl Into<String>) -> ApiError {
    (StatusCode::NOT_FOUND, message.into())
}

fn clean_name(name: &str) -> Result<String, ApiError> {
    let name = name.trim();

    if name.is_empty() {
        return Err(bad_request("Name must not be empty"));
    }

    if name.chars().count() > 60 {
        return Err(bad_request("Name must be 60 characters or fewer"));
    }

    Ok(name.to_string())
}

fn validate_size(width: f64, depth: f64, height: f64) -> Result<(), ApiError> {
    if !(2.0..=60.0).contains(&width) || !(2.0..=60.0).contains(&depth) {
        return Err(bad_request(
            "Width and depth must be between 2 and 60 metres",
        ));
    }

    if !(2.0..=10.0).contains(&height) {
        return Err(bad_request("Height must be between 2 and 10 metres"));
    }

    Ok(())
}

/* -------------------------------------------------------------------------- */
/* Building tree                                                              */
/* -------------------------------------------------------------------------- */

async fn get_building(State(state): State<AppState>) -> Json<crate::blueprints::Building> {
    let store = state.blueprints.read().await;
    Json(store.building.clone())
}

#[derive(Debug, Deserialize)]
struct CreateFloorInput {
    name: String,
}

async fn create_floor(
    State(state): State<AppState>,
    Json(input): Json<CreateFloorInput>,
) -> Result<Json<crate::blueprints::Floor>, ApiError> {
    let name = clean_name(&input.name)?;

    let mut store = state.blueprints.write().await;

    let floor = crate::blueprints::Floor {
        id: store.building.next_floor_id(),
        name,
        rooms: Vec::new(),
    };

    store.building.floors.push(floor.clone());
    persist(&store).await?;

    Ok(Json(floor))
}

async fn delete_floor(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let room_ids = {
        let store = state.blueprints.read().await;

        store
            .building
            .find_floor(&id)
            .map(|floor| {
                floor
                    .rooms
                    .iter()
                    .map(|room| room.id.clone())
                    .collect::<Vec<_>>()
            })
            .ok_or_else(|| not_found("Floor not found"))?
    };

    for room_id in &room_ids {
        simulators::stop_room(&state, room_id).await;
        forget_room_topics(&state, room_id).await;
    }

    let mut store = state.blueprints.write().await;
    store.building.floors.retain(|floor| floor.id != id);
    store
        .devices
        .retain(|device| !room_ids.contains(&device.room_id));
    persist(&store).await?;

    Ok(Json(
        json!({ "status": "deleted", "rooms": room_ids.len() }),
    ))
}

#[derive(Debug, Deserialize)]
struct CreateRoomInput {
    name: String,
    kind: Option<RoomKind>,
    color: Option<String>,
    width: Option<f64>,
    depth: Option<f64>,
    height: Option<f64>,
}

async fn create_room(
    State(state): State<AppState>,
    Path(floor_id): Path<String>,
    Json(input): Json<CreateRoomInput>,
) -> Result<Json<Room>, ApiError> {
    let name = clean_name(&input.name)?;

    let width = input.width.unwrap_or(6.0);
    let depth = input.depth.unwrap_or(5.0);
    let height = input.height.unwrap_or(3.0);
    validate_size(width, depth, height)?;

    let mut store = state.blueprints.write().await;

    if store.building.find_floor(&floor_id).is_none() {
        return Err(not_found("Floor not found"));
    }

    let room_id = format!("room-{}", &Uuid::new_v4().simple().to_string()[..8]);

    let mut room = Room::new(room_id, name, input.kind.unwrap_or(RoomKind::Office));
    room.width = width;
    room.depth = depth;
    room.height = height;

    if let Some(color) = input.color {
        room.color = color;
    }

    let floor = store
        .building
        .find_floor_mut(&floor_id)
        .ok_or_else(|| not_found("Floor not found"))?;

    floor.rooms.push(room.clone());

    blueprints::ensure_room_devices(&mut store, &floor_id, &room);
    persist(&store).await?;
    drop(store);

    simulators::spawn_room(&state, floor_id, room.id.clone()).await;

    Ok(Json(room))
}

#[derive(Debug, Deserialize)]
struct RoomUpdateInput {
    name: Option<String>,
    color: Option<String>,
    width: Option<f64>,
    depth: Option<f64>,
    height: Option<f64>,
    kind: Option<RoomKind>,
}

async fn update_room(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(input): Json<RoomUpdateInput>,
) -> Result<Json<Room>, ApiError> {
    let mut store = state.blueprints.write().await;

    let floor_id = store
        .building
        .floor_id_for_room(&room_id)
        .ok_or_else(|| not_found("Room not found"))?;

    let room = store
        .building
        .find_room_mut(&room_id)
        .ok_or_else(|| not_found("Room not found"))?;

    if let Some(name) = input.name {
        room.name = clean_name(&name)?;
    }

    if let Some(color) = &input.color {
        let color = color.trim();

        if !color.is_empty() && color.chars().count() <= 32 {
            room.color = color.to_string();
        }
    }

    let width = input.width.unwrap_or(room.width);
    let depth = input.depth.unwrap_or(room.depth);
    let height = input.height.unwrap_or(room.height);
    validate_size(width, depth, height)?;

    room.width = width;
    room.depth = depth;
    room.height = height;

    if let Some(kind) = input.kind {
        room.kind = kind;
    }

    let snapshot = room.clone();

    blueprints::ensure_room_devices(&mut store, &floor_id, &snapshot);
    persist(&store).await?;

    Ok(Json(snapshot))
}

async fn delete_room(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    simulators::stop_room(&state, &room_id).await;
    forget_room_topics(&state, &room_id).await;

    let mut store = state.blueprints.write().await;

    let floor_id = store
        .building
        .floor_id_for_room(&room_id)
        .ok_or_else(|| not_found("Room not found"))?;

    if let Some(floor) = store.building.find_floor_mut(&floor_id) {
        floor.rooms.retain(|room| room.id != room_id);
    }

    store.devices.retain(|device| device.room_id != room_id);

    state.presence.write().await.remove(&room_id);

    persist(&store).await?;

    Ok(Json(json!({ "status": "deleted" })))
}

/* -------------------------------------------------------------------------- */
/* Player presence (3D walk mode)                                             */
/* -------------------------------------------------------------------------- */

#[derive(Debug, Deserialize)]
struct PresenceInput {
    player_id: String,
    room_id: Option<String>,
}

async fn set_presence(
    State(state): State<AppState>,
    Json(input): Json<PresenceInput>,
) -> Result<Json<Value>, ApiError> {
    let player_id = input.player_id.trim().to_string();

    if player_id.is_empty() {
        return Err(bad_request("player_id must not be empty"));
    }

    let target = match &input.room_id {
        Some(room_id) => {
            let store = state.blueprints.read().await;

            let floor_id = store
                .building
                .floor_id_for_room(room_id)
                .ok_or_else(|| not_found("Room not found"))?;

            Some((floor_id, room_id.clone()))
        }
        None => None,
    };

    let mut presence = state.presence.write().await;

    let previous = presence
        .iter()
        .find(|(_, players)| players.contains(&player_id))
        .map(|(room_id, _)| room_id.clone());

    if previous.as_deref() == target.as_ref().map(|(_, room_id)| room_id.as_str()) {
        let players = target
            .as_ref()
            .map(|(_, room_id)| {
                presence
                    .get(room_id)
                    .map(|players| players.len())
                    .unwrap_or(0)
            })
            .unwrap_or(0);

        return Ok(Json(json!({
            "room_id": target.as_ref().map(|(_, room_id)| room_id.clone()),
            "players": players
        })));
    }

    if let Some(previous_room) = previous {
        let now_empty = match presence.get_mut(&previous_room) {
            Some(players) => {
                players.remove(&player_id);
                players.is_empty()
            }
            None => false,
        };

        if now_empty {
            presence.remove(&previous_room);
        }
    }

    let mut players_in_target = 0;

    if let Some((_, room_id)) = &target {
        presence
            .entry(room_id.clone())
            .or_default()
            .insert(player_id.clone());

        players_in_target = presence
            .get(room_id)
            .map(|players| players.len())
            .unwrap_or(0);
    }

    drop(presence);

    // Immediate sensor reaction when a player walks into a room.
    if let Some((floor_id, room_id)) = &target {
        if players_in_target > 0 {
            simulators::publish_presence_entered(&state.mqtt, floor_id, room_id, players_in_target)
                .await;
        }

        publish_presence_event(&state, floor_id, room_id, players_in_target, true).await;

        return Ok(Json(json!({
            "room_id": room_id,
            "players": players_in_target
        })));
    }

    Ok(Json(json!({ "room_id": Value::Null, "players": 0 })))
}

async fn publish_presence_event(
    state: &AppState,
    floor_id: &str,
    room_id: &str,
    players: usize,
    occupied: bool,
) {
    let message = MqttMessage {
        timestamp: chrono::Utc::now(),
        device_id: "presence-simulator".to_string(),
        room_id: Some(room_id.to_string()),
        message_type: "presence".to_string(),
        value: json!({
            "players": players,
            "occupied": occupied
        }),
        metadata: Map::from_iter([("reason".to_string(), json!("player_presence"))]),
    };

    let _ = state
        .mqtt
        .publish(
            topics::room_event(floor_id, room_id),
            rumqttc::QoS::AtLeastOnce,
            false,
            serde_json::to_vec(&message).unwrap(),
        )
        .await;
}

/* -------------------------------------------------------------------------- */
/* Manual actuator control (3D walk mode)                                     */
/* -------------------------------------------------------------------------- */

#[derive(Debug, Deserialize)]
struct ControlInput {
    enabled: bool,
}

async fn control_actuator(
    State(state): State<AppState>,
    Path((room_id, actuator)): Path<(String, String)>,
    Json(input): Json<ControlInput>,
) -> Result<Json<Value>, ApiError> {
    let floor_id = {
        let store = state.blueprints.read().await;

        let floor_id = store
            .building
            .floor_id_for_room(&room_id)
            .ok_or_else(|| not_found("Room not found"))?;

        let known = store.devices.iter().any(|device| {
            device.room_id == room_id
                && device.kind == DeviceKind::Actuator
                && device.device_type == actuator
        });

        if !known {
            return Err(not_found(format!("Unknown actuator: {actuator}")));
        }

        floor_id
    };

    let message = MqttMessage {
        timestamp: chrono::Utc::now(),
        device_id: "game-controller".to_string(),
        room_id: Some(room_id.clone()),
        message_type: "command".to_string(),
        value: json!({
            "enabled": input.enabled,
            "mode": "game"
        }),
        metadata: Map::from_iter([
            ("reason".to_string(), json!("manual_override")),
            (
                "correlation_id".to_string(),
                json!(format!("cmd-{}", chrono::Utc::now().timestamp_millis())),
            ),
        ]),
    };

    state
        .mqtt
        .publish(
            topics::actuator_command(&floor_id, &room_id, &actuator),
            rumqttc::QoS::AtLeastOnce,
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
        "status": "command published",
        "room": room_id,
        "actuator": actuator,
        "enabled": input.enabled
    })))
}

/* -------------------------------------------------------------------------- */
/* Shared helpers                                                             */
/* -------------------------------------------------------------------------- */

/// Removes the cached MQTT state of a deleted room from `/api/state`.
async fn forget_room_topics(state: &AppState, room_id: &str) {
    let marker = format!("/room/{room_id}/");

    let simulator_devices = [
        format!("sensor-{room_id}"),
        format!("actuator-{room_id}"),
        format!("controller-{room_id}"),
        format!("energy-meter-{room_id}"),
    ];

    let mut topics = state.topics.write().await;

    topics.retain(|topic, _| {
        let owned_by_simulator = simulator_devices
            .iter()
            .any(|device| topic.ends_with(&format!("/device/{device}/availability")));

        !topic.contains(&marker) && !owned_by_simulator
    });
}

async fn persist(store: &BlueprintStore) -> Result<(), ApiError> {
    crate::storage::save_store(store).await.map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not save building: {error}"),
        )
    })
}
