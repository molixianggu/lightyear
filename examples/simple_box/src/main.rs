#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(dead_code)]

//! Run with
//! - `cargo run -- server`
//! - `cargo run -- client -c 1`
mod client;
mod protocol;

#[cfg(not(target_family = "wasm"))]
mod server;
mod shared;

use async_compat::Compat;
use std::net::{Ipv4Addr, SocketAddr};
use std::str::FromStr;

use bevy::log::{Level, LogPlugin};
use bevy::prelude::*;
use bevy::tasks::IoTaskPool;
use bevy::DefaultPlugins;
use bevy_inspector_egui::quick::WorldInspectorPlugin;
use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};

use crate::client::ClientPluginGroup;
#[cfg(not(target_family = "wasm"))]
use crate::server::ServerPluginGroup;
use lightyear::connection::netcode::{ClientId, Key};
use lightyear::prelude::TransportConfig;
use lightyear::shared::log::add_log_layer;

// Use a port of 0 to automatically select a port
pub const CLIENT_PORT: u16 = 0;
pub const SERVER_PORT: u16 = 5000;
pub const PROTOCOL_ID: u64 = 0;

pub const KEY: Key = [0; 32];

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum Transports {
    #[cfg(not(target_family = "wasm"))]
    Udp,
    WebTransport,
    WebSocket,
}

#[derive(Parser, PartialEq, Debug)]
enum Cli {
    SinglePlayer,
    #[cfg(not(target_family = "wasm"))]
    Server {
        #[arg(long, default_value = "false")]
        headless: bool,

        #[arg(short, long, default_value = "false")]
        inspector: bool,

        #[arg(short, long, default_value_t = SERVER_PORT)]
        port: u16,

        #[arg(short, long, value_enum, default_value_t = Transports::WebTransport)]
        transport: Transports,
    },
    Client {
        #[arg(short, long, default_value = "false")]
        inspector: bool,

        #[arg(short, long, default_value_t = 0)]
        client_id: u64,

        #[arg(long, default_value_t = CLIENT_PORT)]
        client_port: u16,

        #[arg(long, default_value_t = Ipv4Addr::LOCALHOST)]
        server_addr: Ipv4Addr,

        #[arg(short, long, default_value_t = SERVER_PORT)]
        server_port: u16,

        #[arg(short, long, value_enum, default_value_t = Transports::WebTransport)]
        transport: Transports,
    },
}

cfg_if::cfg_if! {
    if #[cfg(target_family = "wasm")] {
        fn main() {
            // NOTE: clap argument parsing does not work on WASM
            let client_id = rand::random::<u64>();
            let cli = Cli::Client {
                inspector: false,
                client_id,
                client_port: CLIENT_PORT,
                server_addr: Ipv4Addr::LOCALHOST,
                server_port: SERVER_PORT,
                transport: Transports::WebTransport,
            };
            let mut app = App::new();
            setup_client(&mut app, cli);
            app.run();
        }
    } else {
        fn main() {
            let cli = Cli::parse();
            let mut app = App::new();
            setup(&mut app, cli);
            app.run();
        }
    }
}

fn setup(app: &mut App, cli: Cli) {
    match cli {
        Cli::SinglePlayer => {}
        #[cfg(not(target_family = "wasm"))]
        Cli::Server {
            headless,
            inspector,
            port,
            transport,
        } => {
            if !headless {
                app.add_plugins(DefaultPlugins.build().disable::<LogPlugin>());
            } else {
                app.add_plugins(MinimalPlugins);
            }
            app.add_plugins(LogPlugin {
                level: Level::INFO,
                filter: "wgpu=error,bevy_render=info,bevy_ecs=trace".to_string(),
                update_subscriber: Some(add_log_layer),
            });

            if inspector {
                app.add_plugins(WorldInspectorPlugin::new());
            }
            // this is async because we need to load the certificate from io
            // we need async_compat because wtransport expects a tokio reactor
            let server_plugin_group = IoTaskPool::get()
                .scope(|s| {
                    s.spawn(Compat::new(async {
                        ServerPluginGroup::new(port, transport, headless).await
                    }));
                })
                .pop()
                .unwrap();
            app.add_plugins(server_plugin_group.build());
        }
        Cli::Client { .. } => {
            setup_client(app, cli);
        }
    }
}

fn setup_client(app: &mut App, cli: Cli) {
    let Cli::Client {
        inspector,
        client_id,
        client_port,
        server_addr,
        server_port,
        transport,
    } = cli
    else {
        return;
    };
    // NOTE: create the default plugins first so that the async task pools are initialized
    // use the default bevy logger for now
    // (the lightyear logger doesn't handle wasm)
    app.add_plugins(DefaultPlugins.set(LogPlugin {
        level: Level::INFO,
        filter: "wgpu=error,bevy_render=info,bevy_ecs=trace".to_string(),
        update_subscriber: Some(add_log_layer),
    }));

    if inspector {
        app.add_plugins(WorldInspectorPlugin::new());
    }
    let server_addr = SocketAddr::new(server_addr.into(), server_port);
    let client_plugin_group =
        ClientPluginGroup::new(client_id, client_port, server_addr, transport);
    app.add_plugins(client_plugin_group.build());
}
