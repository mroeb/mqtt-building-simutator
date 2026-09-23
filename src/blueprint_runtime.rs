use crate::{
    BUILDING_ID, MqttMessage,
    blueprints::{Blueprint, BlueprintEdge, BlueprintNode, BlueprintStore},
};
use rumqttc::{AsyncClient, QoS};
use serde_json::{Map, Value, json};
use std::collections::HashMap;

pub async fn execute_blueprints(
    client: &AsyncClient,
    store: &BlueprintStore,
    topic: &str,
    message: &MqttMessage,
) {
    for blueprint in &store.blueprints {
        if !blueprint.enabled {
            continue;
        }

        execute_blueprint(client, store, blueprint, topic, message).await;
    }
}

async fn execute_blueprint(
    client: &AsyncClient,
    store: &BlueprintStore,
    blueprint: &Blueprint,
    changed_topic: &str,
    message: &MqttMessage,
) {
    let mut values: HashMap<String, Value> = HashMap::new();

    for node in &blueprint.nodes {
        if node.node_type == "sensor" {
            let Some(device_id) = node.data["device_id"].as_str() else {
                continue;
            };

            let Some(device) = store.devices.iter().find(|d| d.id == device_id) else {
                continue;
            };

            if device.mqtt_topic == changed_topic {
                let sensor_value = extract_sensor_value(message);
                values.insert(format!("{}:value", node.id), sensor_value);
            }
        }

        if node.node_type == "constant_number" {
            values.insert(format!("{}:value", node.id), node.data["value"].clone());
        }

        if node.node_type == "constant_boolean" {
            values.insert(format!("{}:value", node.id), node.data["value"].clone());
        }
    }

    // A production-grade version should topologically sort the graph.
    // This MVP resolves nodes repeatedly so simple chains work.
    for _ in 0..blueprint.nodes.len() {
        for node in &blueprint.nodes {
            evaluate_node(node, &blueprint.edges, &mut values);
        }
    }

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

        let Some(device) = store.devices.iter().find(|d| d.id == device_id) else {
            continue;
        };

        if !device.enabled {
            continue;
        }

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
            ]),
        };

        let _ = client
            .publish(
                &device.mqtt_topic,
                QoS::AtLeastOnce,
                false,
                serde_json::to_vec(&command).unwrap(),
            )
            .await;
    }
}

fn evaluate_node(
    node: &BlueprintNode,
    edges: &[BlueprintEdge],
    values: &mut HashMap<String, Value>,
) {
    match node.node_type.as_str() {
        "compare_greater" => {
            let left = get_input_number(node, edges, values, "a");
            let right = get_input_number(node, edges, values, "b");

            if let (Some(left), Some(right)) = (left, right) {
                values.insert(format!("{}:result", node.id), json!(left > right));
            }
        }

        "compare_less" => {
            let left = get_input_number(node, edges, values, "a");
            let right = get_input_number(node, edges, values, "b");

            if let (Some(left), Some(right)) = (left, right) {
                values.insert(format!("{}:result", node.id), json!(left < right));
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

fn extract_sensor_value(message: &MqttMessage) -> Value {
    for field in ["temperature_c", "humidity_percent", "co2_ppm", "occupied"] {
        if let Some(value) = message.value.get(field) {
            return value.clone();
        }
    }

    Value::Null
}
