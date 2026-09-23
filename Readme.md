# MQTT Smart Building Simulator

A Rust-based smart-building simulation that demonstrates an MQTT system for room climate control, energy management and visual automation.

The application simulates multiple office rooms with sensors and actuators, manages MQTT communication, provides a live web dashboard, and includes a Blueprint-inspired visual scripting editor for automation rules.

![Rust](https://img.shields.io/badge/Rust-2024%20edition-orange)
![MQTT](https://img.shields.io/badge/MQTT-Mosquitto-blue)
![Dashboard](https://img.shields.io/badge/Dashboard-Axum-green)

---

## Features

### Building simulation

- **Building → Floor → Room** hierarchy, editable at runtime:
  - Default: `Floor 1` with `room-101` and `room-102`
  - Add/remove floors and rooms from the **Building** tab
  - Customise every room: name, colour, width, depth, height and kind
    (`office` or `garage`)
- Every room runs four independent simulator tasks that start and stop with
  the room: sensors, actuators, controller and energy meter.
- Simulated sensors:
  - Temperature
  - Humidity
  - CO₂
  - Occupancy
  - Motion
  - Illuminance (follows the room's lights)
  - Energy meter (watt + kWh, integrated from actuator states)
- Simulated actuators:
  - Heating
  - Ventilation
  - Lights
  - Door lock
  - Garage door (animates between 0 % and 100 % open, draws power while moving)

### 3D walk minigame

- Walk through the building in first person (Three.js, **3D Walk** tab):
  - `W A S D` + mouse look, `Shift` to run, `E` to interact
  - Orbit view for an overview of the selected floor
- Walking into a room triggers its **occupancy** and **motion** sensors
  immediately, which the Blueprint scripts react to (for example lights).
- `E` interacts with the nearest device: door lock, lights, garage door —
  published as ordinary MQTT actuator commands.
- A locked door or a closed garage shutter physically blocks the doorway.

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

- Live room overview, generated from the building tree.
- Temperature, humidity, CO₂, occupancy, motion and illuminance indicators.
- Per-room **power (W)** and **energy (kWh)** plus building totals.
- Actuator status lamps including door lock and garage door.
- Registered-device activity list.
- MQTT availability display.
- Event log.
- Automation statistics.
- Blueprint script overview.
- **3D Walk** tab with the walkable building simulation.
- **Building** tab for floor/room management and room customisation.

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
             │ - Occupancy         │ │ - Door lock            │
             │ - Motion            │ │ - Garage door          │
             │ - Illuminance       │ │ - State confirmation   │
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
│   ├── main.rs                 # HTTP API, MQTT clients, Blueprint endpoints
│   ├── blueprint_runtime.rs    # Blueprint execution engine
│   ├── blueprints.rs           # Building/Floor/Room + device/graph structures
│   ├── building_api.rs         # Building tree, presence and game control API
│   ├── simulators.rs           # Per-room sensor/actuator/controller/energy tasks
│   ├── topics.rs               # Central MQTT topic builder
│   └── storage.rs              # JSON persistence
└── web/
    └── index.html              # Dashboard, building editor + 3D walk (embedded)
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

All MQTT topics use lowercase words and `/`-separated levels and follow the
**Building → Floor → Room** structure. Every topic is built centrally in
`src/topics.rs`.

```text
building/{building_id}/floor/{floor_id}/room/{room_id}/sensor/{sensor_type}
building/{building_id}/floor/{floor_id}/room/{room_id}/actuator/{actuator_type}/command
building/{building_id}/floor/{floor_id}/room/{room_id}/actuator/{actuator_type}/state
building/{building_id}/floor/{floor_id}/room/{room_id}/config
building/{building_id}/floor/{floor_id}/room/{room_id}/event
building/{building_id}/device/{device_id}/availability
```

### Examples

```text
building/main/floor/floor-1/room/room-101/sensor/temperature
building/main/floor/floor-1/room/room-101/sensor/co2
building/main/floor/floor-1/room/room-101/sensor/occupancy
building/main/floor/floor-1/room/room-101/sensor/motion
building/main/floor/floor-1/room/room-101/sensor/illuminance
building/main/floor/floor-1/room/room-101/sensor/energy

building/main/floor/floor-1/room/room-101/actuator/heating/command
building/main/floor/floor-1/room/room-101/actuator/heating/state
building/main/floor/floor-1/room/room-101/actuator/door_lock/command
building/main/floor/floor-1/room/room-101/actuator/lights/state

building/main/floor/floor-1/room/room-101/config
building/main/floor/floor-1/room/room-101/event

building/main/device/sensor-room-101/availability
building/main/device/actuator-room-101/availability
building/main/device/energy-meter-room-101/availability
```

### Default devices per room

| Kind | Sensors | Actuators |
|---|---|---|
| `office` | temperature, humidity, co2, occupancy, motion, illuminance, energy | heating, ventilation, lights, door_lock |
| `garage` | same as office | same as office + **garage_door** |

Devices are added automatically when a room is created, with deterministic
ids such as `room-101-sensor-temperature` so the built-in scripts can find
them.

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
building/main/floor/floor-1/room/room-101/actuator/heating/state
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

## Energy meter

Every room runs an energy-meter task that subscribes to all retained actuator
states of that room, integrates their power draw once per second and publishes
the result every two seconds:

```text
building/main/floor/floor-1/room/room-101/sensor/energy
```

```json
{
  "power_w": 1497.5,
  "energy_kwh": 0.0415
}
```

Rated power per actuator (scaled by `power_percent`): heating 2000 W,
ventilation 800 W, lights 250 W, garage door 700 W, door lock 100 W.

The dashboard shows per-room and building-wide totals. Energy totals are held
in memory and reset when the application restarts.

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
| `GET` | `/api/building` | Full Building → Floor → Room tree |
| `POST` | `/api/floors` | Create a floor |
| `DELETE` | `/api/floors/{id}` | Delete a floor and all of its rooms |
| `POST` | `/api/floors/{id}/rooms` | Create a room (starts its simulators) |
| `PATCH` | `/api/rooms/{id}` | Customise name, colour, size or kind |
| `DELETE` | `/api/rooms/{id}` | Delete a room and stop its simulators |
| `POST` | `/api/presence` | Report which room a player is walking in |
| `POST` | `/api/room/{room}/actuator/{actuator}` | Manual/game actuator command |
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

Add a floor and a garage room:

```bash
curl -X POST http://localhost:3000/api/floors \
  -H "Content-Type: application/json" \
  -d "{\"name\": \"Floor 2\"}"

curl -X POST http://localhost:3000/api/floors/floor-2/rooms \
  -H "Content-Type: application/json" \
  -d "{\"name\": \"Garage East\", \"kind\": \"garage\", \"color\": \"#a85f5f\"}"
```

Customise a room (all fields optional):

```bash
curl -X PATCH http://localhost:3000/api/rooms/room-101 \
  -H "Content-Type: application/json" \
  -d "{\"name\": \"Blue Lab\", \"width\": 8.5, \"depth\": 6.0, \"height\": 3.4}"
```

Report that a player walked into a room (triggers occupancy and motion):

```bash
curl -X POST http://localhost:3000/api/presence \
  -H "Content-Type: application/json" \
  -d "{\"player_id\": \"player-1\", \"room_id\": \"room-101\"}"
```

Open the garage door or lock a door (same API the game uses):

```bash
curl -X POST http://localhost:3000/api/room/room-101/actuator/door_lock \
  -H "Content-Type: application/json" \
  -d "{\"enabled\": true}"

curl -X POST http://localhost:3000/api/room/room-101/actuator/garage_door \
  -H "Content-Type: application/json" \
  -d "{\"enabled\": true}"
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

- Dynamic devices can be registered but only the default per-room devices run
  as independent simulator tasks.
- Energy totals and events are held in memory and reset on restart.
- The 3D view renders the rooms of the selected floor only; there are no
  stairs, players walk through floor switching instead.
- The 3D view loads Three.js from a CDN, so it needs an internet connection.
- Blueprint evaluation uses repeated resolution rather than a topological graph execution engine.
- There is no authentication or user-role management.
- MQTT uses local-development settings without TLS or credentials.
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
- Add energy calculation in kWh: **done** — per-room energy meter with
  watt and kWh readings.
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
- Add a building floor-plan visualization: **done** — 3D walk view with a
  doll-house overview of each floor.
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
5. Show independent sensor values for both rooms plus power/energy totals.
6. Show actuator status lamps, door lock and garage door states.
7. Open the **Building** tab, add a floor and a garage room, and watch its
   simulators come online in the device list.
8. Open the **3D Walk** tab, walk into a room and show occupancy/motion
   triggering the lights Blueprint, then lock the door with `E`.
9. Open the Blueprint Scripts tab.
10. Demonstrate:
    - Occupancy Sensor → Lights
    - Temperature Sensor → Comparison → Heating
    - CO₂ Sensor → Comparison → Ventilation
11. Show Blueprint execution events.
12. Explain QoS 0, QoS 1 and QoS 2.
13. Restart the dashboard to show retained actuator state.
14. Demonstrate availability and LWT messages.
15. Show saved Blueprint data in `data/blueprints.json`.
