use crate::protocol::*;
use crate::shared::{shared_config, shared_movement_behaviour};
use crate::{shared, Cli, Transports, KEY, PROTOCOL_ID};
use bevy::app::PluginGroupBuilder;
use bevy::prelude::*;
use bevy::utils::Duration;
use leafwing_input_manager::plugin::InputManagerSystem;
use leafwing_input_manager::prelude::*;
use leafwing_input_manager::systems::{run_if_enabled, tick_action_state};
use lightyear::_reexport::ShouldBeInterpolated;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::str::FromStr;

pub struct ClientPluginGroup {
    client_id: ClientId,
    lightyear: ClientPlugin<MyProtocol>,
}

impl ClientPluginGroup {
    pub(crate) fn new(
        client_id: u64,
        client_port: u16,
        server_addr: SocketAddr,
        transport: Transports,
    ) -> ClientPluginGroup {
        let auth = Authentication::Manual {
            server_addr,
            client_id,
            private_key: KEY,
            protocol_id: PROTOCOL_ID,
        };
        let client_addr = SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), client_port);
        let certificate_digest =
            String::from("2b:08:3b:2a:2b:9a:ad:dc:ed:ba:80:43:c3:1a:43:3e:2c:06:11:a0:61:25:4b:fb:ca:32:0e:5d:85:5d:a7:56")
                .replace(":", "");
        let transport_config = match transport {
            #[cfg(not(target_family = "wasm"))]
            Transports::Udp => TransportConfig::UdpSocket(client_addr),
            Transports::WebTransport => TransportConfig::WebTransportClient {
                client_addr,
                server_addr,
                #[cfg(target_family = "wasm")]
                certificate_digest,
            },
            Transports::WebSocket => TransportConfig::WebSocketClient { server_addr },
        };
        let link_conditioner = LinkConditionerConfig {
            incoming_latency: Duration::from_millis(100),
            incoming_jitter: Duration::from_millis(10),
            incoming_loss: 0.00,
        };
        let config = ClientConfig {
            shared: shared_config(),
            net: NetConfig::Netcode {
                auth,
                config: NetcodeConfig::default(),
                io: IoConfig::from_transport(transport_config).with_conditioner(link_conditioner),
            },
            prediction: PredictionConfig::default(),
            interpolation: InterpolationConfig::default().with_delay(
                InterpolationDelay::default()
                    .with_min_delay(Duration::from_millis(50))
                    .with_send_interval_ratio(2.0),
            ),
            ..default()
        };
        let plugin_config = PluginConfig::new(config, protocol());
        ClientPluginGroup {
            client_id,
            lightyear: ClientPlugin::new(plugin_config),
        }
    }
}

impl PluginGroup for ClientPluginGroup {
    fn build(self) -> PluginGroupBuilder {
        PluginGroupBuilder::start::<Self>()
            .add(self.lightyear)
            .add(ExampleClientPlugin {
                client_id: self.client_id,
            })
            .add(shared::SharedPlugin)
            .add(LeafwingInputPlugin::<MyProtocol, Inputs>::default())
    }
}

pub struct ExampleClientPlugin {
    client_id: ClientId,
}

#[derive(Resource)]
pub struct Global {
    client_id: ClientId,
}

impl Plugin for ExampleClientPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ActionState<Inputs>>();
        app.insert_resource(Global {
            client_id: self.client_id,
        });
        app.add_systems(Startup, init);
        app.add_systems(FixedUpdate, movement);
        app.add_systems(
            Update,
            (
                add_input_map,
                handle_predicted_spawn,
                handle_interpolated_spawn,
                log,
            ),
        );
    }
}

// Startup system for the client
pub(crate) fn init(mut commands: Commands, mut client: ClientMut, global: Res<Global>) {
    commands.spawn(Camera2dBundle::default());
    commands.spawn(TextBundle::from_section(
        format!("Client {}", global.client_id),
        TextStyle {
            font_size: 30.0,
            color: Color::WHITE,
            ..default()
        },
    ));
    let _ = client.connect();
}

// The client input only gets applied to predicted entities that we own
// This works because we only predict the user's controlled entity.
// If we were predicting more entities, we would have to only apply movement to the player owned one.
pub(crate) fn movement(
    // TODO: maybe make prediction mode a separate component!!!
    mut position_query: Query<(&mut Position, &ActionState<Inputs>), With<Predicted>>,
) {
    // if we are not doing prediction, no need to read inputs
    if <Components as SyncMetadata<Position>>::mode() != ComponentSyncMode::Full {
        return;
    }
    for (position, input) in position_query.iter_mut() {
        shared_movement_behaviour(position, input);
    }
}

// System to receive messages on the client
pub(crate) fn add_input_map(
    mut commands: Commands,
    predicted_players: Query<Entity, (Added<PlayerId>, With<Predicted>)>,
) {
    // we don't want to replicate the ActionState from the server to client, because if we have an ActionState
    // on the Confirmed player it will keep getting replicated to Predicted and will interfere with our inputs
    for player_entity in predicted_players.iter() {
        commands.entity(player_entity).insert((
            PlayerBundle::get_input_map(),
            ActionState::<Inputs>::default(),
        ));
    }
}

// When the predicted copy of the client-owned entity is spawned, do stuff
// - assign it a different saturation
pub(crate) fn handle_predicted_spawn(mut predicted: Query<&mut PlayerColor, Added<Predicted>>) {
    for mut color in predicted.iter_mut() {
        color.0.set_s(0.3);
    }
}

// When the predicted copy of the client-owned entity is spawned, do stuff
// - assign it a different saturation
pub(crate) fn handle_interpolated_spawn(
    mut interpolated: Query<&mut PlayerColor, Added<Interpolated>>,
) {
    for mut color in interpolated.iter_mut() {
        color.0.set_s(0.1);
    }
}

pub(crate) fn log(
    tick_manager: Res<TickManager>,
    connection: Res<ClientConnectionManager>,
    confirmed: Query<&Position, With<Confirmed>>,
    predicted: Query<&Position, (With<Predicted>, Without<Confirmed>)>,
    mut interp_event: EventReader<ComponentInsertEvent<ShouldBeInterpolated>>,
    mut predict_event: EventReader<ComponentInsertEvent<ShouldBePredicted>>,
) {
    let server_tick = connection.latest_received_server_tick();
    for confirmed_pos in confirmed.iter() {
        debug!(?server_tick, "Confirmed position: {:?}", confirmed_pos);
    }
    let client_tick = tick_manager.tick();
    for predicted_pos in predicted.iter() {
        debug!(?client_tick, "Predicted position: {:?}", predicted_pos);
    }
    for event in interp_event.read() {
        info!("Interpolated event: {:?}", event.entity());
    }
    for event in predict_event.read() {
        info!("Predicted event: {:?}", event.entity());
    }
}
