//! Central MQTT topic builder.
//!
//! Every topic in the application is created here so the
//! Building -> Floor -> Room hierarchy only has to be changed in one place.

use crate::BUILDING_ID;

/// `building/{building}`
pub fn building() -> String {
    format!("building/{BUILDING_ID}")
}

/// `building/{building}/floor/{floor}`
pub fn floor(floor_id: &str) -> String {
    format!("{}/floor/{floor_id}", building())
}

/// `building/{building}/floor/{floor}/room/{room}`
pub fn room(floor_id: &str, room_id: &str) -> String {
    format!("{}/room/{room_id}", floor(floor_id))
}

/// `building/{building}/floor/{floor}/room/{room}/sensor/{sensor_type}`
pub fn sensor(floor_id: &str, room_id: &str, sensor_type: &str) -> String {
    format!("{}/sensor/{sensor_type}", room(floor_id, room_id))
}

/// `building/{building}/floor/{floor}/room/{room}/actuator/{actuator}/command`
pub fn actuator_command(floor_id: &str, room_id: &str, actuator: &str) -> String {
    format!("{}/actuator/{actuator}/command", room(floor_id, room_id))
}

/// `building/{building}/floor/{floor}/room/{room}/actuator/{actuator}/state`
pub fn actuator_state(floor_id: &str, room_id: &str, actuator: &str) -> String {
    format!("{}/actuator/{actuator}/state", room(floor_id, room_id))
}

/// `building/{building}/floor/{floor}/room/{room}/actuator/+/command`
pub fn actuator_command_filter(floor_id: &str, room_id: &str) -> String {
    format!("{}/actuator/+/command", room(floor_id, room_id))
}

/// `building/{building}/floor/{floor}/room/{room}/actuator/+/state`
pub fn actuator_state_filter(floor_id: &str, room_id: &str) -> String {
    format!("{}/actuator/+/state", room(floor_id, room_id))
}

/// `building/{building}/floor/{floor}/room/{room}/event`
pub fn room_event(floor_id: &str, room_id: &str) -> String {
    format!("{}/event", room(floor_id, room_id))
}

/// `building/{building}/floor/{floor}/room/{room}/config`
pub fn room_config(floor_id: &str, room_id: &str) -> String {
    format!("{}/config", room(floor_id, room_id))
}

/// `building/{building}/floor/{floor}/room/{room}/#`
pub fn room_filter(floor_id: &str, room_id: &str) -> String {
    format!("{}/#", room(floor_id, room_id))
}

/// `building/{building}/device/{device_id}/availability`
pub fn device_availability(device_id: &str) -> String {
    format!("{}/device/{device_id}/availability", building())
}

/// `building/{building}/#`
pub fn building_filter() -> String {
    format!("{}/#", building())
}

/// Extracts the actuator name from a command or state topic.
///
/// `building/main/floor/floor-1/room/room-101/actuator/heating/command`
/// returns `heating`.
pub fn actuator_from_topic(topic: &str) -> Option<String> {
    let parts: Vec<&str> = topic.split('/').collect();

    if parts.len() >= 9 && parts[6] == "actuator" {
        Some(parts[7].to_string())
    } else {
        None
    }
}
