use crate::{
    BUILDING_ID, MqttMessage,
    blueprints::{Blueprint, BlueprintEdge, BlueprintNode, BlueprintStore, DeviceKind},
};
use rumqttc::{AsyncClient, QoS};
use serde_json::{Map, Value, json};
use std::collections::HashMap;

/// Runs enabled scripts after a sensor measurement arrives.
///
/// `topic_cache` contains the latest message for every MQTT topic,
/// so a graph can use multiple sensors in one evaluation.
pub async fn execute_blueprints(
    client: &AsyncClient,
    store: &BlueprintStore,
    topic_cache: &HashMap<String, MqttMessage>,
) {
    for blueprint in &store.blueprints {
        if !blueprint.enabled {
            continue;
        }

        execute_blueprint(client, store, blueprint, topic_cache).await;
    }
}

async fn execute_blueprint(
    client: &AsyncClient,
    store: &BlueprintStore,
    blueprint: &Blueprint,
    topic_cache: &HashMap<String, MqttMessage>,
) {
    let mut values: HashMap<String, Value> = HashMap::new();

    /* Seed all source nodes. */
    for node in &blueprint.nodes {
        match node.node_type.as_str() {
            "sensor" => {
                let Some(device_id) = node.data["device_id"].as_str() else {
                    continue;
                };

                let Some(device) = store
                    .devices
                    .iter()
                    .find(|device| device.id == device_id && device.kind == DeviceKind::Sensor)
                else {
                    continue;
                };

                let Some(message) = topic_cache.get(&device.mqtt_topic) else {
                    continue;
                };

                if let Some(value) = extract_sensor_value(message) {
                    values.insert(format!("{}:value", node.id), value);
                }
            }

            "constant_number" | "constant_boolean" => {
                if let Some(value) = node.data.get("value") {
                    values.insert(format!("{}:value", node.id), value.clone());
                }
            }

            _ => {}
        }
    }

    /*
     * Resolve logic nodes repeatedly.
     * This supports normal acyclic Blueprint-style chains without
     * needing a topological sort implementation yet.
     */
    for _ in 0..blueprint.nodes.len() {
        for node in &blueprint.nodes {
            evaluate_node(node, &blueprint.edges, &mut values);
        }
    }

    /* Send commands to configured actuator nodes. */
    for node in &blueprint.nodes {
        if node.node_type != "actuator" {
            continue;
        }

        let Some(enabled) = get_input_boolean(node, &blueprint.edges, &values, "enabled") else {
            continue;
        };

        let Some(device_id) = node.data["device_id"].as_str() else {
            continue;
        };

        let Some(device) = store.devices.iter().find(|device| {
            device.id == device_id && device.kind == DeviceKind::Actuator && device.enabled
        }) else {
            continue;
        };

        let command = MqttMessage {
            timestamp: chrono::Utc::now(),
            device_id: format!("blueprint-{}", blueprint.id),
            room_id: Some(device.room_id.clone()),
            message_type: "command".to_string(),
            value: json!({
                "enabled": enabled,
                "mode": "blueprint"
            }),
            metadata: Map::from_iter([
                ("blueprint_id".to_string(), json!(blueprint.id)),
                ("blueprint_name".to_string(), json!(blueprint.name)),
                ("reason".to_string(), json!("blueprint_graph_evaluation")),
            ]),
        };

        match client
            .publish(
                &device.mqtt_topic,
                QoS::AtLeastOnce,
                false,
                serde_json::to_vec(&command).unwrap(),
            )
            .await
        {
            Ok(()) => {
                // Publish a dashboard-visible event only after the MQTT command
                // was successfully queued for publication.
                publish_blueprint_event(
                    client,
                    blueprint,
                    &device.room_id,
                    &device.device_type,
                    enabled,
                )
                .await;
            }

            Err(error) => {
                eprintln!(
                    "Blueprint '{}' could not publish actuator command: {error}",
                    blueprint.name
                );
            }
        }
    }
}

fn evaluate_node(
    node: &BlueprintNode,
    edges: &[BlueprintEdge],
    values: &mut HashMap<String, Value>,
) {
    match node.node_type.as_str() {
        "compare_greater" => {
            let a = get_input_number(node, edges, values, "a");
            let b = get_input_number(node, edges, values, "b");

            if let (Some(a), Some(b)) = (a, b) {
                values.insert(format!("{}:result", node.id), json!(a > b));
            }
        }

        "compare_less" => {
            let a = get_input_number(node, edges, values, "a");
            let b = get_input_number(node, edges, values, "b");

            if let (Some(a), Some(b)) = (a, b) {
                values.insert(format!("{}:result", node.id), json!(a < b));
            }
        }

        "boolean_and" => {
            let a = get_input_boolean(node, edges, values, "a");
            let b = get_input_boolean(node, edges, values, "b");

            if let (Some(a), Some(b)) = (a, b) {
                values.insert(format!("{}:result", node.id), json!(a && b));
            }
        }

        "boolean_or" => {
            let a = get_input_boolean(node, edges, values, "a");
            let b = get_input_boolean(node, edges, values, "b");

            if let (Some(a), Some(b)) = (a, b) {
                values.insert(format!("{}:result", node.id), json!(a || b));
            }
        }

        _ => {}
    }
}

fn get_input_value(
    node: &BlueprintNode,
    edges: &[BlueprintEdge],
    values: &HashMap<String, Value>,
    input_handle: &str,
) -> Option<Value> {
    let edge = edges
        .iter()
        .find(|edge| edge.target == node.id && edge.target_handle == input_handle)?;

    values
        .get(&format!("{}:{}", edge.source, edge.source_handle))
        .cloned()
}

fn get_input_number(
    node: &BlueprintNode,
    edges: &[BlueprintEdge],
    values: &HashMap<String, Value>,
    input_handle: &str,
) -> Option<f64> {
    get_input_value(node, edges, values, input_handle)?.as_f64()
}

fn get_input_boolean(
    node: &BlueprintNode,
    edges: &[BlueprintEdge],
    values: &HashMap<String, Value>,
    input_handle: &str,
) -> Option<bool> {
    get_input_value(node, edges, values, input_handle)?.as_bool()
}

fn extract_sensor_value(message: &MqttMessage) -> Option<Value> {
    for field in ["temperature_c", "humidity_percent", "co2_ppm", "occupied"] {
        if let Some(value) = message.value.get(field) {
            return Some(value.clone());
        }
    }

    None
}

async fn publish_blueprint_event(
    client: &AsyncClient,
    blueprint: &Blueprint,
    room_id: &str,
    actuator: &str,
    enabled: bool,
) {
    let event = MqttMessage {
        timestamp: chrono::Utc::now(),
        device_id: format!("blueprint-{}", blueprint.id),
        room_id: Some(room_id.to_string()),
        message_type: "blueprint_execution".to_string(),
        value: json!({
            "actuator": actuator,
            "enabled": enabled
        }),
        metadata: Map::from_iter([
            ("blueprint_id".to_string(), json!(blueprint.id)),
            ("blueprint_name".to_string(), json!(blueprint.name)),
        ]),
    };

    let topic = format!("building/{BUILDING_ID}/room/{room_id}/event");

    let _ = client
        .publish(
            topic,
            QoS::AtLeastOnce,
            false,
            serde_json::to_vec(&event).unwrap(),
        )
        .await;
}
