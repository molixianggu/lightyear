use crate::client::resource::Client;
use crate::prelude::Protocol;
use crate::transport::io::{IoDiagnosticsPlugin, IoStats};
use bevy::app::{App, Plugin, PostUpdate};
use bevy::diagnostic::{Diagnostic, Diagnostics, RegisterDiagnostic};
use bevy::prelude::{Real, Res, ResMut, Time};

pub struct ClientDiagnosticsPlugin<P> {
    _marker: std::marker::PhantomData<P>,
}

impl<P> Default for ClientDiagnosticsPlugin<P> {
    fn default() -> Self {
        Self {
            _marker: std::marker::PhantomData,
        }
    }
}

fn io_diagnostics_system<P: Protocol>(
    mut client: ResMut<Client<P>>,
    time: Res<Time<Real>>,
    mut diagnostics: Diagnostics,
) {
    IoDiagnosticsPlugin::update_diagnostics(&mut client.io.stats, &time, &mut diagnostics);
}
impl<P: Protocol> Plugin for ClientDiagnosticsPlugin<P> {
    fn build(&self, app: &mut App) {
        app.add_plugins(IoDiagnosticsPlugin::default());
        app.add_systems(PostUpdate, io_diagnostics_system::<P>);
    }
}
