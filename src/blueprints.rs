use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Sensor,
    Actuator,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub room_id: String,
    pub name: String,
    pub kind: DeviceKind,

    /// Examples: temperature, co2, humidity, occupancy, heating, lights.
    pub device_type: String,

    /// Used to create a unique MQTT topic.
    pub mqtt_topic: String,

    pub enabled: bool,
}

impl Device {
    pub fn new(room_id: String, name: String, kind: DeviceKind, device_type: String) -> Self {
        let id = format!("device-{}", Uuid::new_v4());

        let mqtt_topic = match kind {
            DeviceKind::Sensor => format!("building/main/room/{room_id}/sensor/{device_type}"),
            DeviceKind::Actuator => {
                format!("building/main/room/{room_id}/actuator/{device_type}/command")
            }
        };

        Self {
            id,
            room_id,
            name,
            kind,
            device_type,
            mqtt_topic,
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Blueprint {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub nodes: Vec<BlueprintNode>,
    pub edges: Vec<BlueprintEdge>,
}

impl Blueprint {
    pub fn empty(name: String) -> Self {
        Self {
            id: format!("script-{}", Uuid::new_v4()),
            name,
            enabled: true,
            nodes: Vec::new(),
            edges: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueprintNode {
    pub id: String,

    /// sensor, actuator, compare_greater, compare_less,
    /// boolean_and, boolean_or, constant_number, constant_boolean
    pub node_type: String,

    pub position: NodePosition,

    /// Node-specific settings.
    /// For example:
    /// { "device_id": "device-abc" }
    /// { "value": 1000 }
    pub data: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodePosition {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueprintEdge {
    pub id: String,
    pub source: String,
    pub source_handle: String,
    pub target: String,
    pub target_handle: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueprintStore {
    pub devices: Vec<Device>,
    pub blueprints: Vec<Blueprint>,
}

impl Default for BlueprintStore {
    fn default() -> Self {
        Self {
            devices: Vec::new(),
            blueprints: Vec::new(),
        }
    }
}
