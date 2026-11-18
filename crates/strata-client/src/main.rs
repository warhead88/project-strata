use bevy::log::{Level, LogPlugin};
use bevy::prelude::*;

fn main() -> AppExit {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Project Strata".into(),
                        ..default()
                    }),
                    ..default()
                })
                .set(LogPlugin {
                    level: Level::ERROR,
                    filter: "strata_client=info".into(),
                    ..default()
                }),
        )
        .insert_resource(ClearColor(Color::srgb(0.05, 0.08, 0.10)))
        .add_systems(Startup, setup_camera)
        .run()
}

fn setup_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
}
