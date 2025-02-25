//! Defines the server bevy systems and run conditions
use bevy::ecs::system::SystemChangeTick;
use bevy::prelude::*;
use tracing::{debug, error, trace, trace_span};

use crate::_reexport::ComponentProtocol;
use crate::connection::server::{NetServer, ServerConnection};
use crate::prelude::{MainSet, TickManager, TimeManager};
use crate::protocol::message::MessageProtocol;
use crate::protocol::Protocol;
use crate::server::connection::ConnectionManager;
use crate::server::events::{ConnectEvent, DisconnectEvent, EntityDespawnEvent, EntitySpawnEvent};
use crate::server::room::RoomManager;
use crate::shared::events::connection::{IterEntityDespawnEvent, IterEntitySpawnEvent};
use crate::shared::replication::ReplicationSend;
use crate::shared::time_manager::is_ready_to_send;

pub(crate) struct ServerNetworkingPlugin<P: Protocol> {
    marker: std::marker::PhantomData<P>,
}
impl<P: Protocol> Default for ServerNetworkingPlugin<P> {
    fn default() -> Self {
        Self {
            marker: std::marker::PhantomData,
        }
    }
}

impl<P: Protocol> Plugin for ServerNetworkingPlugin<P> {
    fn build(&self, app: &mut App) {
        app
            // SYSTEM SETS
            .configure_sets(PreUpdate, (MainSet::Receive, MainSet::ReceiveFlush).chain())
            .configure_sets(
                PostUpdate,
                (
                    // we don't send packets every frame, but on a timer instead
                    MainSet::Send.run_if(is_ready_to_send),
                    MainSet::SendPackets.in_set(MainSet::Send),
                ),
            )
            // SYSTEMS //
            .add_systems(
                PreUpdate,
                (
                    receive::<P>.in_set(MainSet::Receive),
                    apply_deferred.in_set(MainSet::ReceiveFlush),
                ),
            )
            .add_systems(PostUpdate, (send::<P>.in_set(MainSet::SendPackets),));
    }
}

pub(crate) fn receive<P: Protocol>(world: &mut World) {
    trace!("Receive client packets");
    world.resource_scope(|world: &mut World, mut connection_manager: Mut<ConnectionManager<P>>| {
        world.resource_scope(
            |world: &mut World, mut netcode: Mut<ServerConnection>| {
                    world.resource_scope(
                        |world: &mut World, mut time_manager: Mut<TimeManager>| {
                            world.resource_scope(
                                |world: &mut World, tick_manager: Mut<TickManager>| {
                                    world.resource_scope(
                                        |world: &mut World, mut room_manager: Mut<RoomManager>| {
                                            let delta = world.resource::<Time<Virtual>>().delta();
                                            // UPDATE: update server state, send keep-alives, receive packets from io
                                            // update time manager
                                            time_manager.update(delta);
                                            trace!(time = ?time_manager.current_time(), tick = ?tick_manager.tick(), "receive");

                                            // update netcode server
                                            let _ = netcode
                                                .try_update(delta.as_secs_f64())
                                                .map_err(|e| error!("Error updating netcode server: {:?}", e));

                                            // update connections
                                            connection_manager
                                                .update(time_manager.as_ref(), tick_manager.as_ref());

                                            // handle connection
                                            for client_id in netcode.new_connections().iter().copied() {
                                                // let client_addr = self.netcode.client_addr(client_id).unwrap();
                                                // info!("New connection from {} (id: {})", client_addr, client_id);
                                                connection_manager.add(client_id);
                                            }

                                            // handle disconnections
                                            for client_id in netcode.new_disconnections().iter().copied() {
                                                connection_manager.remove(client_id);
                                                room_manager.client_disconnect(client_id);
                                            };

                                            // update connections
                                            connection_manager
                                                .update(time_manager.as_ref(), tick_manager.as_ref());

                                            // RECV_PACKETS: buffer packets into message managers
                                            while let Some((mut reader, client_id)) = netcode.recv() {
                                                // TODO: use connection to apply on BOTH message manager and replication manager
                                                connection_manager
                                                    .connection_mut(client_id)
                                                    .expect("connection not found")
                                                    .recv_packet(&mut reader, tick_manager.as_ref())
                                                    .expect("could not recv packet");
                                            }

                                            // RECEIVE: read messages and parse them into events
                                            connection_manager
                                                .receive(world, time_manager.as_ref(), tick_manager.as_ref())
                                                .unwrap_or_else(|e| {
                                                    error!("Error during receive: {}", e);
                                                });

                                            // EVENTS: Write the received events into bevy events
                                            if !connection_manager.events.is_empty() {
                                                // TODO: write these as systems? might be easier to also add the events to the app
                                                //  it might just be less efficient? + maybe tricky to
                                                // Input events
                                                // Update the input buffers with any InputMessage received:

                                                // ADD A FUNCTION THAT ITERATES THROUGH EACH CONNECTION AND RETURNS InputEvent for THE CURRENT TICK

                                                // Connection / Disconnection events
                                                if connection_manager.events.has_connections() {
                                                    let mut connect_event_writer =
                                                        world.get_resource_mut::<Events<ConnectEvent>>().unwrap();
                                                    for client_id in connection_manager.events.iter_connections() {
                                                        debug!("Client connected event: {}", client_id);
                                                        connect_event_writer.send(ConnectEvent::new(client_id));
                                                    }
                                                }

                                                if connection_manager.events.has_disconnections() {
                                                    let mut connect_event_writer =
                                                        world.get_resource_mut::<Events<DisconnectEvent>>().unwrap();
                                                    for client_id in connection_manager.events.iter_disconnections() {
                                                        debug!("Client disconnected event: {}", client_id);
                                                        connect_event_writer.send(DisconnectEvent::new(client_id));
                                                    }
                                                }

                                                // Message Events
                                                P::Message::push_message_events(world, &mut connection_manager.events);

                                                // EntitySpawn Events
                                                if connection_manager.events.has_entity_spawn() {
                                                    let mut entity_spawn_event_writer = world
                                                        .get_resource_mut::<Events<EntitySpawnEvent>>()
                                                        .unwrap();
                                                    for (entity, client_id) in connection_manager.events.into_iter_entity_spawn() {
                                                        entity_spawn_event_writer.send(EntitySpawnEvent::new(entity, client_id));
                                                    }
                                                }
                                                // EntityDespawn Events
                                                if connection_manager.events.has_entity_despawn() {
                                                    let mut entity_despawn_event_writer = world
                                                        .get_resource_mut::<Events<EntityDespawnEvent>>()
                                                        .unwrap();
                                                    for (entity, client_id) in connection_manager.events.into_iter_entity_spawn() {
                                                        entity_despawn_event_writer.send(EntityDespawnEvent::new(entity, client_id));
                                                    }
                                                }

                                                // Update component events (updates, inserts, removes)
                                                P::Components::push_component_events(world, &mut connection_manager.events);
                                            }
                                        });
                                });
                        });
                });
    });
}

// or do additional send stuff here
pub(crate) fn send<P: Protocol>(
    change_tick: SystemChangeTick,
    mut netserver: ResMut<ServerConnection>,
    mut connection_manager: ResMut<ConnectionManager<P>>,
    tick_manager: Res<TickManager>,
    time_manager: Res<TimeManager>,
) {
    trace!("Send packets to clients");
    // finalize any packets that are needed for replication
    connection_manager
        .buffer_replication_messages(tick_manager.tick(), change_tick.this_run())
        .unwrap_or_else(|e| {
            error!("Error preparing replicate send: {}", e);
        });

    // SEND_PACKETS: send buffered packets to io
    let span = trace_span!("send_packets").entered();
    connection_manager
        .connections
        .iter_mut()
        .try_for_each(|(client_id, connection)| {
            let client_span =
                trace_span!("send_packets_to_client", client_id = ?client_id).entered();
            for packet_byte in connection.send_packets(&time_manager, &tick_manager)? {
                netserver.send(packet_byte.as_slice(), *client_id)?;
            }
            Ok(())
        })
        .unwrap_or_else(|e: anyhow::Error| {
            error!("Error sending packets: {}", e);
        });

    // clear the list of newly connected clients
    // (cannot just use the ConnectionEvent because it is cleared after each frame)
    connection_manager.new_clients.clear();
}

/// Clear the received events
/// We put this in a separate as send because we want to run this every frame, and
/// Send only runs every send_interval
pub(crate) fn clear_events<P: Protocol>(mut connection_manager: ResMut<ConnectionManager<P>>) {
    connection_manager.events.clear();
}
