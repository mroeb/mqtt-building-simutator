use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::BUILDING_ID;
use crate::topics;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Sensor,
    Actuator,
}

/* -------------------------------------------------------------------------- */
/* Building -> Floor -> Room                                                  */
/* -------------------------------------------------------------------------- */

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RoomKind {
    Office,
    Garage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Room {
    pub id: String,
    pub name: String,

    /// Wall colour used by the 3D view.
    pub color: String,

    /// Room size in metres.
    pub width: f64,
    pub depth: f64,
    pub height: f64,

    pub kind: RoomKind,
}

impl Room {
    pub fn new(id: String, name: String, kind: RoomKind) -> Self {
        Self {
            id,
            name,
            color: "#2f6f9f".to_string(),
            width: 6.0,
            depth: 5.0,
            height: 3.0,
            kind,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Floor {
    pub id: String,
    pub name: String,
    pub rooms: Vec<Room>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Building {
    pub id: String,
    pub name: String,
    pub floors: Vec<Floor>,
}

impl Default for Building {
    fn default() -> Self {
        Self {
            id: BUILDING_ID.to_string(),
            name: "Main Building".to_string(),
            floors: Vec::new(),
        }
    }
}

impl Building {
    pub fn find_floor(&self, floor_id: &str) -> Option<&Floor> {
        self.floors.iter().find(|floor| floor.id == floor_id)
    }

    pub fn find_floor_mut(&mut self, floor_id: &str) -> Option<&mut Floor> {
        self.floors.iter_mut().find(|floor| floor.id == floor_id)
    }

    pub fn find_room(&self, room_id: &str) -> Option<(&Floor, &Room)> {
        self.floors.iter().find_map(|floor| {
            floor
                .rooms
                .iter()
                .find(|room| room.id == room_id)
                .map(|room| (floor, room))
        })
    }

    pub fn find_room_mut(&mut self, room_id: &str) -> Option<&mut Room> {
        self.floors
            .iter_mut()
            .find_map(|floor| floor.rooms.iter_mut().find(|room| room.id == room_id))
    }

    pub fn floor_id_for_room(&self, room_id: &str) -> Option<String> {
        self.find_room(room_id).map(|(floor, _)| floor.id.clone())
    }

    /// All `(floor, room)` pairs of the building.
    pub fn rooms(&self) -> Vec<(&Floor, &Room)> {
        self.floors
            .iter()
            .flat_map(|floor| floor.rooms.iter().map(move |room| (floor, room)))
            .collect()
    }

    /// Creates an unused id such as `floor-3`.
    pub fn next_floor_id(&self) -> String {
        let mut index = self.floors.len() + 1;

        while self
            .floors
            .iter()
            .any(|floor| floor.id == format!("floor-{index}"))
        {
            index += 1;
        }

        format!("floor-{index}")
    }
}

/* -------------------------------------------------------------------------- */
/* Devices                                                                    */
/* -------------------------------------------------------------------------- */

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub room_id: String,
    pub name: String,
    pub kind: DeviceKind,

    /// Examples: temperature, co2, occupancy, motion, illuminance,
    /// energy, heating, lights, door_lock, garage_door.
    pub device_type: String,

    /// Used to create a unique MQTT topic.
    pub mqtt_topic: String,

    pub enabled: bool,
}

impl Device {
    /// Random-id device for the Devices tab.
    pub fn for_room(
        floor_id: &str,
        room_id: &str,
        name: String,
        kind: DeviceKind,
        device_type: String,
    ) -> Self {
        Self {
            id: format!("device-{}", Uuid::new_v4()),
            mqtt_topic: topic_for(&kind, floor_id, room_id, &device_type),
            room_id: room_id.to_string(),
            name,
            kind,
            device_type,
            enabled: true,
        }
    }

    /// Deterministic-id device used for the default device set.
    pub fn default_for(
        floor_id: &str,
        room: &Room,
        kind: DeviceKind,
        device_type: &str,
        label: &str,
    ) -> Self {
        let id = match kind {
            DeviceKind::Sensor => format!("{}-sensor-{device_type}", room.id),
            DeviceKind::Actuator => format!("{}-actuator-{device_type}", room.id),
        };

        Self {
            mqtt_topic: topic_for(&kind, floor_id, &room.id, device_type),
            id,
            room_id: room.id.clone(),
            name: format!("{label} ({})", room.name),
            kind,
            device_type: device_type.to_string(),
            enabled: true,
        }
    }
}

fn topic_for(kind: &DeviceKind, floor_id: &str, room_id: &str, device_type: &str) -> String {
    match kind {
        DeviceKind::Sensor => topics::sensor(floor_id, room_id, device_type),
        DeviceKind::Actuator => topics::actuator_command(floor_id, room_id, device_type),
    }
}

/// Default sensors of every room.
pub const ROOM_SENSORS: &[(&str, &str)] = &[
    ("temperature", "Temperature Sensor"),
    ("humidity", "Humidity Sensor"),
    ("co2", "CO₂ Sensor"),
    ("occupancy", "Occupancy Sensor"),
    ("motion", "Motion Sensor"),
    ("illuminance", "Illuminance Sensor"),
    ("energy", "Energy Meter"),
];

/// Default actuators of every room.
pub const ROOM_ACTUATORS: &[(&str, &str)] = &[
    ("heating", "Heating"),
    ("ventilation", "Ventilation"),
    ("lights", "Lights"),
    ("door_lock", "Door Lock"),
];

/// Extra actuators for garage rooms.
pub const GARAGE_ACTUATORS: &[(&str, &str)] = &[("garage_door", "Garage Door")];

/// Adds all missing default devices for one room.
pub fn ensure_room_devices(store: &mut BlueprintStore, floor_id: &str, room: &Room) {
    let mut wanted: Vec<(DeviceKind, &str, &str)> = ROOM_SENSORS
        .iter()
        .map(|(device_type, label)| (DeviceKind::Sensor, *device_type, *label))
        .collect();

    wanted.extend(
        ROOM_ACTUATORS
            .iter()
            .map(|(device_type, label)| (DeviceKind::Actuator, *device_type, *label)),
    );

    if room.kind == RoomKind::Garage {
        wanted.extend(
            GARAGE_ACTUATORS
                .iter()
                .map(|(device_type, label)| (DeviceKind::Actuator, *device_type, *label)),
        );
    }

    for (kind, device_type, label) in wanted {
        let device = Device::default_for(floor_id, room, kind.clone(), device_type, label);

        if !store
            .devices
            .iter()
            .any(|existing| existing.id == device.id)
        {
            store.devices.push(device);
        }
    }
}

/// Recomputes the MQTT topic of every device from the current building tree.
pub fn refresh_device_topics(store: &mut BlueprintStore) {
    for device in &mut store.devices {
        if let Some(floor_id) = store.building.floor_id_for_room(&device.room_id) {
            device.mqtt_topic = topic_for(
                &device.kind,
                &floor_id,
                &device.room_id,
                &device.device_type,
            );
        }
    }
}

/* -------------------------------------------------------------------------- */
/* Blueprints                                                                 */
/* -------------------------------------------------------------------------- */

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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct BlueprintStore {
    pub building: Building,
    pub devices: Vec<Device>,
    pub blueprints: Vec<Blueprint>,
}

/* -------------------------------------------------------------------------- */
/* Defaults and migration                                                     */
/* -------------------------------------------------------------------------- */

/// Two offices on the first floor, used when no building exists yet.
pub fn default_building() -> Building {
    Building {
        id: BUILDING_ID.to_string(),
        name: "Main Building".to_string(),
        floors: vec![Floor {
            id: "floor-1".to_string(),
            name: "Floor 1".to_string(),
            rooms: vec![
                Room::new(
                    "room-101".to_string(),
                    "Office 101".to_string(),
                    RoomKind::Office,
                ),
                Room::new(
                    "room-102".to_string(),
                    "Office 102".to_string(),
                    RoomKind::Office,
                ),
            ],
        }],
    }
}

/// Repairs stores written by older versions of the application:
///
/// * creates the default building when none exists
/// * removes devices that point at deleted rooms
/// * rebuilds all MQTT topics from the current building tree
pub fn migrate_store(store: &mut BlueprintStore) {
    if store.building.floors.is_empty() {
        store.building = default_building();
    }

    let known_rooms: Vec<String> = store
        .building
        .rooms()
        .into_iter()
        .map(|(_, room)| room.id.clone())
        .collect();

    store
        .devices
        .retain(|device| known_rooms.contains(&device.room_id));

    refresh_device_topics(store);
}

/// Adds the default devices and the three built-in example scripts
/// for every room that does not have them yet.
pub fn ensure_default_content(store: &mut BlueprintStore) {
    let rooms: Vec<(String, Room)> = store
        .building
        .rooms()
        .into_iter()
        .map(|(floor, room)| (floor.id.clone(), room.clone()))
        .collect();

    for (floor_id, room) in rooms {
        ensure_room_devices(store, &floor_id, &room);
        ensure_builtin_scripts(store, &room);
    }
}

fn ensure_builtin_scripts(store: &mut BlueprintStore, room: &Room) {
    let room_id = &room.id;

    let heating_script_id = format!("builtin-temperature-heating-{room_id}");

    if !store
        .blueprints
        .iter()
        .any(|script| script.id == heating_script_id)
    {
        store.blueprints.push(Blueprint {
            id: heating_script_id,
            name: format!("{}: Heat when temperature is below 21.5 °C", room.name),
            enabled: true,
            nodes: vec![
                node(
                    "temperature-sensor",
                    "sensor",
                    80.0,
                    120.0,
                    json!({ "device_id": format!("{room_id}-sensor-temperature") }),
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
                    json!({ "device_id": format!("{room_id}-actuator-heating") }),
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

    let lights_script_id = format!("builtin-occupancy-lights-{room_id}");

    if !store
        .blueprints
        .iter()
        .any(|script| script.id == lights_script_id)
    {
        store.blueprints.push(Blueprint {
            id: lights_script_id,
            name: format!("{}: Turn lights on when occupied", room.name),
            enabled: true,
            nodes: vec![
                node(
                    "occupancy-sensor",
                    "sensor",
                    100.0,
                    180.0,
                    json!({ "device_id": format!("{room_id}-sensor-occupancy") }),
                ),
                node(
                    "lights-output",
                    "actuator",
                    480.0,
                    180.0,
                    json!({ "device_id": format!("{room_id}-actuator-lights") }),
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

    let ventilation_script_id = format!("builtin-co2-ventilation-{room_id}");

    if !store
        .blueprints
        .iter()
        .any(|script| script.id == ventilation_script_id)
    {
        store.blueprints.push(Blueprint {
            id: ventilation_script_id,
            name: format!("{}: Ventilate when CO₂ is above 1000 ppm", room.name),
            enabled: true,
            nodes: vec![
                node(
                    "co2-sensor",
                    "sensor",
                    80.0,
                    120.0,
                    json!({ "device_id": format!("{room_id}-sensor-co2") }),
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
                    json!({ "device_id": format!("{room_id}-actuator-ventilation") }),
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
