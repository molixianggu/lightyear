use crate::protocol::*;
use crate::shared::shared_config;
use crate::{shared, Transports, KEY, PROTOCOL_ID};
use bevy::app::PluginGroupBuilder;
use bevy::ecs::archetype::Archetype;
use bevy::prelude::*;
use leafwing_input_manager::prelude::ActionState;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::ops::Deref;
use std::time::Duration;

// Plugin group to add all server-related plugins
pub struct ServerPluginGroup {
    pub(crate) lightyear: ServerPlugin<MyProtocol>,
}

impl ServerPluginGroup {
    pub(crate) async fn new(port: u16, transport: Transports) -> ServerPluginGroup {
        // Step 1: create the io (transport + link conditioner)
        let server_addr = SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), port);
        let transport_config = match transport {
            Transports::Udp => TransportConfig::UdpSocket(server_addr),
            // if using webtransport, we load the certificate keys
            Transports::WebTransport => {
                let certificate =
                    Certificate::load("../certificates/cert.pem", "../certificates/key.pem")
                        .await
                        .unwrap();
                let digest = &certificate.hashes()[0];
                println!("Generated self-signed certificate with digest: {}", digest);
                TransportConfig::WebTransportServer {
                    server_addr,
                    certificate,
                }
            }
            Transports::WebSocket => TransportConfig::WebSocketServer { server_addr },
        };
        let link_conditioner = LinkConditionerConfig {
            incoming_latency: Duration::from_millis(0),
            incoming_jitter: Duration::from_millis(0),
            incoming_loss: 0.0,
        };

        // Step 2: define the server configuration
        let config = ServerConfig {
            shared: shared_config().clone(),
            packet: PacketConfig::default()
                // by default there is no bandwidth limit so we need to enable it
                .enable_bandwidth_cap()
                // we can set the max bandwidth to 56 KB/s
                .with_send_bandwidth_bytes_per_second_cap(1500),
            net: NetConfig::Netcode {
                config: NetcodeConfig::default()
                    .with_protocol_id(PROTOCOL_ID)
                    .with_key(KEY),
                io: IoConfig::from_transport(transport_config).with_conditioner(link_conditioner),
            },
            ping: PingConfig::default(),
        };

        // Step 3: create the plugin
        let plugin_config = PluginConfig::new(config, protocol());
        ServerPluginGroup {
            lightyear: ServerPlugin::new(plugin_config),
        }
    }
}

impl PluginGroup for ServerPluginGroup {
    fn build(self) -> PluginGroupBuilder {
        PluginGroupBuilder::start::<Self>()
            .add(self.lightyear)
            .add(ExampleServerPlugin)
            .add(shared::SharedPlugin)
    }
}

// Plugin for server-specific logic
pub struct ExampleServerPlugin;

impl Plugin for ExampleServerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Global>();
        app.add_systems(Startup, init);
        // the physics/FixedUpdates systems that consume inputs should be run in this set
        app.add_plugins(LeafwingInputPlugin::<MyProtocol, Inputs>::default());
        app.add_systems(
            Update,
            (handle_connections, log, (tick_timers, update_props).chain()),
        );
    }
}

const GRID_SIZE: f32 = 20.0;
const NUM_CIRCLES: i32 = 6;

#[derive(Resource, Default)]
pub(crate) struct Global {
    pub client_id_to_entity_id: HashMap<ClientId, Entity>,
    pub client_id_to_room_id: HashMap<ClientId, RoomId>,
}

pub(crate) fn init(mut commands: Commands) {
    commands.spawn(Camera2dBundle::default());
    commands.spawn(TextBundle::from_section(
        "Server",
        TextStyle {
            font_size: 30.0,
            color: Color::WHITE,
            ..default()
        },
    ));

    // spawn dots in a grid
    for x in -NUM_CIRCLES..NUM_CIRCLES {
        for y in -NUM_CIRCLES..NUM_CIRCLES {
            commands.spawn((
                Position(Vec2::new(x as f32 * GRID_SIZE, y as f32 * GRID_SIZE)),
                Shape::Circle,
                ShapeChangeTimer(Timer::from_seconds(2.0, TimerMode::Repeating)),
                Replicate {
                    // A ReplicationGroup is replicated together as a single message, so the priority should
                    // be set on the group.
                    // A group with priority 2.0 will be replicated twice as often as a group with priority 1.0
                    // in case the bandwidth is saturated.
                    // The priority can be sent when the entity is spawned; if multiple entities in the same group have
                    // different priorities, the latest set priority will be used.
                    // After the entity is spawned, you can update the priority using the ConnectionManager::upate_priority method.
                    replication_group: ReplicationGroup::default()
                        .set_priority(1.0 + y.abs() as f32),
                    ..default()
                },
            ));
        }
    }
}

/// Server connection system, create a player upon connection
pub(crate) fn handle_connections(
    mut connections: EventReader<ConnectEvent>,
    mut disconnections: EventReader<DisconnectEvent>,
    mut global: ResMut<Global>,
    mut commands: Commands,
) {
    for connection in connections.read() {
        let client_id = connection.context();
        // Generate pseudo random color from client id.
        let h = (((client_id.wrapping_mul(30)) % 360) as f32) / 360.0;
        let s = 0.8;
        let l = 0.5;
        let entity = commands.spawn(PlayerBundle::new(
            *client_id,
            Vec2::splat(300.0),
            Color::hsl(h, s, l),
        ));
        // Add a mapping from client id to entity id (so that when we receive an input from a client,
        // we know which entity to move)
        global
            .client_id_to_entity_id
            .insert(*client_id, entity.id());
    }
    for disconnection in disconnections.read() {
        let client_id = disconnection.context();
        if let Some(entity) = global.client_id_to_entity_id.remove(client_id) {
            commands.entity(entity).despawn();
        }
    }
}

pub(crate) fn tick_timers(mut timers: Query<&mut ShapeChangeTimer>, time: Res<Time>) {
    for mut timer in timers.iter_mut() {
        timer.tick(time.delta());
    }
}

pub(crate) fn update_props(mut props: Query<(&mut Shape, &ShapeChangeTimer)>) {
    for (mut shape, timer) in props.iter_mut() {
        if timer.just_finished() {
            if shape.deref() == &Shape::Circle {
                *shape = Shape::Triangle;
            } else if shape.deref() == &Shape::Triangle {
                *shape = Shape::Square;
            } else if shape.deref() == &Shape::Square {
                *shape = Shape::Circle;
            }
        }
    }
}

pub(crate) fn log(tick_manager: Res<TickManager>, position: Query<&Position, With<PlayerId>>) {
    let server_tick = tick_manager.tick();
    for pos in position.iter() {
        debug!(?server_tick, "Confirmed position: {:?}", pos);
    }
}
