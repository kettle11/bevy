//! Shows how to render simple primitive shapes with a single color.

use bevy::{
    prelude::*,
    sprite::{MaterialMesh2dBundle, Mesh2dHandle},
};

fn main() {
    bevy::winit::WinitOffscreen::init(|| {
        App::new()
            .add_plugins(
                DefaultPlugins
                    .set(bevy::log::LogPlugin {
                        filter: "info,wgpu_core=debug,wgpu_hal=debug,mygame=debug".into(),
                        level: bevy::log::Level::DEBUG,
                        ..Default::default()
                    })
                    .set(WindowPlugin {
                        primary_window: Some(Window {
                            // provide the ID selector string here
                            canvas: Some("#canvas".into()),
                            // ... any other window properties ...
                            ..default()
                        }),
                        ..default()
                    }),
            )
            .add_systems(Startup, setup)
            .run();
    })
    .transfer_canvas_to_offscreen("#canvas");
}

const X_EXTENT: f32 = 900.;

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    info!("SETTING UP!");
    commands.spawn(Camera2dBundle::default());

    let shapes = [
        Mesh2dHandle(meshes.add(Circle { radius: 50.0 })),
        Mesh2dHandle(meshes.add(CircularSector::new(50.0, 1.0))),
        Mesh2dHandle(meshes.add(CircularSegment::new(50.0, 1.25))),
        Mesh2dHandle(meshes.add(Ellipse::new(25.0, 50.0))),
        Mesh2dHandle(meshes.add(Annulus::new(25.0, 50.0))),
        Mesh2dHandle(meshes.add(Capsule2d::new(25.0, 50.0))),
        Mesh2dHandle(meshes.add(Rhombus::new(75.0, 100.0))),
        Mesh2dHandle(meshes.add(Rectangle::new(50.0, 100.0))),
        Mesh2dHandle(meshes.add(RegularPolygon::new(50.0, 6))),
        Mesh2dHandle(meshes.add(Triangle2d::new(
            Vec2::Y * 50.0,
            Vec2::new(-50.0, -50.0),
            Vec2::new(50.0, -50.0),
        ))),
    ];
    let num_shapes = shapes.len();

    for (i, shape) in shapes.into_iter().enumerate() {
        // Distribute colors evenly across the rainbow.
        let color = Color::hsl(360. * i as f32 / num_shapes as f32, 0.95, 0.7);

        commands.spawn(MaterialMesh2dBundle {
            mesh: shape,
            material: materials.add(color),
            transform: Transform::from_xyz(
                // Distribute shapes from -X_EXTENT/2 to +X_EXTENT/2.
                -X_EXTENT / 2. + i as f32 / (num_shapes - 1) as f32 * X_EXTENT,
                0.0,
                0.0,
            ),
            ..default()
        });
    }
}
