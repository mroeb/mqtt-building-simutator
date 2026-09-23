# MQTT Smart Building Simulator

A Rust-based smart-building simulation that demonstrates an MQTT system for room climate control, energy management and visual automation.

The application simulates multiple office rooms with sensors and actuators, manages MQTT communication, provides a live web dashboard, and includes a Blueprint-inspired visual scripting editor for automation rules.

![Rust](https://img.shields.io/badge/Rust-2024%20edition-orange)
![MQTT](https://img.shields.io/badge/MQTT-Mosquitto-blue)
![Dashboard](https://img.shields.io/badge/Dashboard-Axum-green)

---

## Features

### Building simulation

- Multiple simulated rooms:
  - `room-101`
  - `room-102`
- Independent room simulations with different sensor phases.
- Simulated sensors:
  - Temperature
  - Humidity
  - CO₂
  - Occupancy
- Simulated actuators:
  - Heating
  - Ventilation
  - Lights

### MQTT concepts

- **QoS 0** for frequent sensor telemetry.
- **QoS 1** for actuator commands and state confirmations.
- **QoS 2** for configuration changes.
- Retained actuator-state messages.
- Last Will and Testament for availability monitoring.
- Online/offline availability messages.
- Structured JSON MQTT payloads.
- Separate topics for sensors, commands, state, events, availability and configuration.

### Web dashboard

- Live room overview.
- Temperature, humidity, CO₂ and occupancy indicators.
- Actuator status lamps.
- Registered-device activity list.
- MQTT availability display.
- Event log.
- Automation statistics.
- Blueprint script overview.

### Visual scripting / Blueprint editor

- Blueprint-inspired visual rule editor.
- Draggable nodes.
- Typed connection pins:
  - Blue: numeric values
  - Purple: boolean values
- Sensor and actuator nodes.
- Number and boolean constants.
- Comparison nodes:
  - `A > B`
  - `A < B`
- Boolean nodes:
  - `AND`
  - `OR`
- Server-side persistence of scripts and registered devices.

---

## Architecture

```text
                                ┌──────────────────────────┐
                                │      Web Dashboard       │
                                │       Axum + HTML        │
                                │                          │
                                │ - Live room state        │
                                │ - Device activity        │
                                │ - Blueprint editor       │
                                └────────────┬─────────────┘
                                             │ HTTP REST API
                                             │
                         ┌───────────────────▼───────────────────┐
                         │          Rust Application             │
                         │                                       │
                         │  ┌─────────────────────────────────┐  │
                         │  │ MQTT Dashboard Observer         │  │
                         │  │ - Subscribes to building/#      │  │
                         │  │ - Stores latest topic messages  │  │
                         │  │ - Triggers Blueprint runtime    │  │
                         │  └─────────────────────────────────┘  │
                         │                                       │
                         │  ┌─────────────────────────────────┐  │
                         │  │ Blueprint Runtime               │  │
                         │  │ - Evaluates visual scripts      │  │
                         │  │ - Sends actuator commands       │  │
                         │  └─────────────────────────────────┘  │
                         │                                       │
                         │  ┌─────────────────────────────────┐  │
                         │  │ Persistence                      │  │
                         │  │ data/blueprints.json             │  │
                         │  └─────────────────────────────────┘  │
                         └────────────┬──────────────────────────┘
                                      │ MQTT
                                      ▼
                    ┌─────────────────────────────────┐
                    │          Mosquitto Broker        │
                    │            localhost:1883        │
                    └───────┬───────────────┬─────────┘
                            │               │
             ┌──────────────▼──────┐ ┌──────▼───────────────┐
             │ Sensor Simulators   │ │ Actuator Simulators   │
             │                     │ │                        │
             │ - Temperature       │ │ - Heating              │
             │ - Humidity          │ │ - Ventilation          │
             │ - CO₂               │ │ - Lights               │
             │ - Occupancy         │ │ - State confirmation   │
             └─────────────────────┘ └────────────────────────┘
```

---

## Project structure

```text
mqtt-building-simulator/
├── Cargo.toml
├── Cargo.lock
├── README.md
├── docker-compose.yaml
├── mosquitto.conf
├── data/
│   └── blueprints.json         # Created automatically at runtime
├── src/
│   ├── main.rs                 # HTTP API, MQTT clients and simulation
│   ├── blueprint_runtime.rs    # Blueprint execution engine
│   ├── blueprints.rs           # Device and graph data structures
│   └── storage.rs              # JSON persistence
└── web/
    └── index.html              # Dashboard embedded at compile time
```

The frontend is embedded in the final Rust binary during compilation:

```rust
const DASHBOARD_HTML: &str = include_str!("../web/index.html");
```

The application can therefore serve the dashboard without requiring an external web server or separate frontend files at runtime.

---

## Requirements

- [Rust](https://www.rust-lang.org/tools/install)
- [Docker Desktop](https://www.docker.com/products/docker-desktop/) or a local Mosquitto installation
- A modern browser

Verify the Rust installation:

```bash
rustc --version
cargo --version
```

---

## Quick start

### 1. Clone the repository

```bash
git clone https://github.com/mroeb/mqtt-building-simutator.git
cd mqtt-building-simutator
```

### 2. Start Mosquitto

```bash
docker compose up -d
```

The MQTT broker becomes available at:

```text
mqtt://localhost:1883
```

### 3. Start the application

```bash
cargo run
```

Open the dashboard:

```text
http://localhost:3000
```

---

## Development commands

Run the application:

```bash
cargo run
```

Check compilation without running:

```bash
cargo check
```

Format Rust code:

```bash
cargo fmt
```

Run Clippy:

```bash
cargo clippy
```

Create an optimized release binary:

```bash
cargo build --release
```

The binary is located under:

```text
target/release/
```

On Windows:

```text
target/release/mqtt-building-simulator.exe
```

---

## MQTT topic hierarchy

All MQTT topics use lowercase words and `/`-separated levels.

```text
building/{building_id}/room/{room_id}/sensor/{sensor_type}
building/{building_id}/room/{room_id}/actuator/{actuator_type}/command
building/{building_id}/room/{room_id}/actuator/{actuator_type}/state
building/{building_id}/room/{room_id}/config
building/{building_id}/room/{room_id}/event
building/{building_id}/device/{device_id}/availability
```

### Examples

```text
building/main/room/room-101/sensor/temperature
building/main/room/room-101/sensor/co2
building/main/room/room-101/sensor/occupancy

building/main/room/room-101/actuator/heating/command
building/main/room/room-101/actuator/heating/state

building/main/room/room-101/actuator/lights/command
building/main/room/room-101/actuator/lights/state

building/main/room/room-101/config
building/main/room/room-101/event

building/main/device/sensor-room-101/availability
building/main/device/actuator-room-101/availability
```

---

## MQTT message model

All MQTT messages use a common JSON envelope.

### Sensor measurement

```json
{
  "timestamp": "2026-02-12T10:15:30Z",
  "device_id": "sensor-room-101",
  "room_id": "room-101",
  "message_type": "measurement",
  "value": {
    "temperature_c": 21.4
  },
  "metadata": {
    "simulation": true,
    "sequence": 1842
  }
}
```

### Blueprint actuator command

```json
{
  "timestamp": "2026-02-12T10:15:32Z",
  "device_id": "blueprint-builtin-occupancy-lights-room-101",
  "room_id": "room-101",
  "message_type": "command",
  "value": {
    "enabled": true,
    "mode": "blueprint"
  },
  "metadata": {
    "blueprint_id": "builtin-occupancy-lights-room-101",
    "blueprint_name": "room-101: Turn lights on when occupied",
    "reason": "blueprint_graph_evaluation"
  }
}
```

### Confirmed actuator state

```json
{
  "timestamp": "2026-02-12T10:15:33Z",
  "device_id": "actuator-room-101",
  "room_id": "room-101",
  "message_type": "state",
  "value": {
    "enabled": true,
    "power_percent": 35,
    "mode": "blueprint"
  },
  "metadata": {
    "correlation_id": null
  }
}
```

---

## QoS strategy

| MQTT message type | QoS | Reason |
|---|---:|---|
| Sensor telemetry | 0 | Measurements are sent regularly; losing one is acceptable. |
| Actuator commands | 1 | Commands should arrive at least once. Commands are idempotent. |
| Actuator state | 1 | State confirmation should be reliably delivered. |
| Room configuration | 2 | Configuration changes should be processed exactly once. |
| Availability / LWT | 1 | Device online/offline state should be delivered reliably. |

---

## Retained messages

Actuator state messages are published as retained messages.

Example:

```text
building/main/room/room-101/actuator/heating/state
```

A dashboard that connects later immediately receives the latest known actuator state.

Demonstration:

1. Start the application.
2. Wait for an actuator state update.
3. Refresh or reopen the dashboard.
4. The latest actuator state is available immediately.

---

## Last Will and Testament

Each MQTT client defines a Last Will and Testament message.

If a client disconnects unexpectedly, Mosquitto publishes an offline availability message:

```json
{
  "message_type": "availability",
  "value": {
    "status": "offline",
    "reason": "unexpected_disconnect"
  }
}
```

Availability messages use:

```text
building/main/device/{device_id}/availability
```

---

## Built-in Blueprint scripts

The application generates default simulated devices and scripts during startup if they do not already exist.

### Temperature controls heating

```text
Temperature Sensor → A < B → Heating
                         ↑
                    21.5 °C
```

Expected behavior:

```text
Temperature < 21.5 °C  → Heating ON
Temperature ≥ 21.5 °C  → Heating OFF
```

### Occupancy controls lights

```text
Occupancy Sensor → Lights
```

Expected behavior:

```text
Occupied = true   → Lights ON
Occupied = false  → Lights OFF
```

### CO₂ controls ventilation

```text
CO₂ Sensor → A > B → Ventilation
                 ↑
             1000 ppm
```

Expected behavior:

```text
CO₂ > 1000 ppm   → Ventilation ON
CO₂ ≤ 1000 ppm   → Ventilation OFF
```

---

## Blueprint editor

The visual scripting editor is available in the **Blueprint Scripts** tab.

### Node types

| Node | Input pins | Output pins | Purpose |
|---|---|---|---|
| Sensor | — | `value` | Reads a latest MQTT sensor value |
| Actuator | `enabled` | — | Publishes an actuator command |
| Number constant | — | `value` | Supplies a numeric threshold |
| Boolean constant | — | `value` | Supplies `true` or `false` |
| Greater than | `A`, `B` | `result` | Returns `A > B` |
| Less than | `A`, `B` | `result` | Returns `A < B` |
| Boolean AND | `A`, `B` | `result` | Returns `A AND B` |
| Boolean OR | `A`, `B` | `result` | Returns `A OR B` |

### Pin types

| Pin colour | Type | Examples |
|---|---|---|
| Blue | Number | Temperature, humidity, CO₂, thresholds |
| Purple | Boolean | Occupancy, comparison outputs, actuator enabled state |

Only matching pin types can be connected.

Invalid:

```text
Temperature Sensor → Lights
```

Correct:

```text
Temperature Sensor → A < B → Heating
                         ↑
                 Number Constant
```

---

## Script storage

Registered devices and Blueprint scripts are persisted as JSON:

```text
data/blueprints.json
```

The file is created automatically at startup.

To reset all devices and generated scripts:

### Windows PowerShell

```powershell
Remove-Item .\data\blueprints.json
```

### Linux / macOS

```bash
rm -f data/blueprints.json
```

Restart the application afterward.

> This also removes user-created scripts and devices.

---

## Dashboard REST API

| Method | Endpoint | Description |
|---|---|---|
| `GET` | `/` | Embedded dashboard |
| `GET` | `/api/state` | Latest cached MQTT topic state |
| `GET` | `/api/events` | MQTT events and availability events |
| `GET` | `/api/devices` | Registered Blueprint devices |
| `POST` | `/api/devices` | Register a sensor or actuator |
| `DELETE` | `/api/devices/{id}` | Remove a registered device |
| `GET` | `/api/blueprints` | List Blueprint scripts |
| `POST` | `/api/blueprints` | Create a Blueprint script |
| `POST` | `/api/blueprints/{id}` | Save a Blueprint graph |
| `DELETE` | `/api/blueprints/{id}` | Delete a Blueprint script |
| `POST` | `/api/room/{room}/config` | Publish room configuration |
| `POST` | `/api/room/{room}/failure` | Simulate actuator failure |

---

## Example API requests

Enable Blueprint mode for `room-101`:

```bash
curl -X POST http://localhost:3000/api/room/room-101/config \
  -H "Content-Type: application/json" \
  -d "{\"blueprint_mode\": true}"
```

Enable the normal automatic controller:

```bash
curl -X POST http://localhost:3000/api/room/room-101/config \
  -H "Content-Type: application/json" \
  -d "{\"blueprint_mode\": false}"
```

---

## Control modes

| Mode | Description |
|---|---|
| Blueprint mode | Visual scripts decide actuator states. |
| Automatic controller mode | Built-in Rust rules decide actuator states. |
| Manual mode | Prevents automatic-controller changes. |

The application starts with Blueprint mode enabled:

```rust
let mut blueprint_mode = true;
```

This prevents the automatic controller from overwriting Blueprint commands.

---

## Current limitations

This is an educational simulation project.

- Dynamic devices can be registered but are not automatically started as independent simulator tasks.
- Blueprint evaluation uses repeated resolution rather than a topological graph execution engine.
- There is no authentication or user-role management.
- MQTT uses local-development settings without TLS or credentials.
- Events are stored in memory and are cleared when the application stops.
- There is no full thermal model where heating gradually increases the temperature.
- Blueprint scripts do not yet show live node values or animated signal flow.

---

## Future improvements

### Simulation and control

- Add a physical thermal model:
  - Heating raises temperature gradually.
  - Ventilation lowers CO₂ and affects temperature.
  - Occupancy increases CO₂ over time.
  - Outside temperature affects room temperature.
- Add energy calculation in kWh.
- Add dynamic energy prices and peak-load control.
- Add schedules, night mode and workday profiles.
- Add configurable target temperatures.
- Add energy-saving and comfort profiles.
- Add per-actuator selection of Blueprint, automatic or manual control.

### Blueprint system

- Add live execution visualization:
  - Highlight currently evaluated nodes.
  - Display current pin values.
  - Animate active graph connections.
- Add script enable/disable switches.
- Add graph validation:
  - Missing devices.
  - Required input not connected.
  - Cyclic graph detection.
  - Invalid device/room combinations.
- Add more nodes:
  - Equality comparisons.
  - Greater-or-equal and less-or-equal.
  - `NOT`.
  - Delay/timer.
  - Schedule.
  - Time-of-day.
  - Hysteresis.
  - Rate limiter.
  - MQTT event trigger.
  - Alert/notification.
- Add reusable Blueprint templates.
- Add script version history and rollback.
- Implement topological graph execution.

### MQTT and reliability

- Add MQTT authentication and TLS.
- Add client-specific credentials and access-control lists.
- Add robust reconnect and state recovery.
- Add command acknowledgements with correlation IDs.
- Add deduplication for QoS 1 commands.
- Add broker health monitoring.
- Add dead-letter topics for invalid messages.
- Add environment-variable configuration for MQTT host, port and building ID.

### Dashboard

- Add historical charts for:
  - Temperature
  - Humidity
  - CO₂
  - Energy usage
  - Actuator runtime
- Add a building floor-plan visualization.
- Add animated heating, ventilation and lamp icons.
- Add dark/light theme selection.
- Add room and device filters.
- Add an MQTT topic explorer.
- Add event severity filters.
- Improve mobile layouts.

### Persistence and deployment

- Replace JSON storage with SQLite.
- Store historical telemetry and events.
- Add database migrations.
- Add CSV and JSON export.
- Add a Docker image for the Rust application.
- Add a complete Docker Compose environment.
- Add GitHub Actions CI.
- Add unit, integration and end-to-end tests.

---

## Suggested presentation flow

1. Explain the smart-building scenario.
2. Show the architecture diagram.
3. Start Mosquitto and the Rust application.
4. Open the dashboard.
5. Show independent sensor values for both rooms.
6. Show actuator status lamps.
7. Open the Blueprint Scripts tab.
8. Demonstrate:
   - Occupancy Sensor → Lights
   - Temperature Sensor → Comparison → Heating
   - CO₂ Sensor → Comparison → Ventilation
9. Show Blueprint execution events.
10. Explain QoS 0, QoS 1 and QoS 2.
11. Restart the dashboard to show retained actuator state.
12. Demonstrate availability and LWT messages.
13. Show saved Blueprint data in `data/blueprints.json`.
