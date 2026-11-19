use bevy::prelude::*;

use crate::camera::GlobeCameraState;

/// Marker component for the clickable top-right Compass button that resets camera North-up.
#[derive(Component)]
pub struct CompassButton;

/// Marker component for the rotating North dial inside the Compass button.
#[derive(Component)]
pub struct CompassDial;

/// Marker component for the heading readout label inside the Compass button.
#[derive(Component)]
pub struct CompassHeadingText;

const COMPASS_BG_IDLE: Color = Color::srgba(0.05, 0.08, 0.12, 0.84);
const COMPASS_BG_HOVER: Color = Color::srgba(0.10, 0.16, 0.24, 0.92);
const COMPASS_BG_PRESS: Color = Color::srgba(0.16, 0.28, 0.40, 0.95);
const COMPASS_BORDER_IDLE: Color = Color::srgba(0.45, 0.65, 0.82, 0.65);
const COMPASS_BORDER_HOVER: Color = Color::srgba(0.68, 0.84, 0.98, 0.90);
const COMPASS_BORDER_PRESS: Color = Color::srgb(0.95, 0.78, 0.30);

/// Spawns the clickable UI Compass widget in the top-right corner of the viewport.
pub fn setup_compass_ui(mut commands: Commands) {
    commands
        .spawn((
            CompassButton,
            Pickable::default(),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(12.0),
                right: Val::Px(12.0),
                width: Val::Px(64.0),
                height: Val::Px(64.0),
                border: UiRect::all(Val::Px(1.5)),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: Val::Px(2.0),
                ..default()
            },
            BackgroundColor(COMPASS_BG_IDLE),
            BorderColor::all(COMPASS_BORDER_IDLE),
        ))
        .observe(on_compass_click)
        .observe(on_compass_over)
        .observe(on_compass_out)
        .observe(on_compass_press)
        .observe(on_compass_release)
        .with_children(|parent| {
            parent
                .spawn((
                    CompassDial,
                    Pickable::IGNORE,
                    Node {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                    UiTransform::IDENTITY,
                ))
                .with_children(|dial| {
                    dial.spawn((
                        Pickable::IGNORE,
                        Text::new("▲"),
                        TextFont {
                            font_size: FontSize::Px(14.0),
                            ..default()
                        },
                        TextColor(Color::srgb(1.0, 0.38, 0.25)),
                    ));
                    dial.spawn((
                        Pickable::IGNORE,
                        Text::new("N"),
                        TextFont {
                            font_size: FontSize::Px(13.0),
                            ..default()
                        },
                        TextColor(Color::srgb(0.95, 0.97, 1.0)),
                    ));
                });

            parent.spawn((
                CompassHeadingText,
                Pickable::IGNORE,
                Text::new("0°"),
                TextFont {
                    font_size: FontSize::Px(11.0),
                    ..default()
                },
                TextColor(Color::srgb(0.72, 0.82, 0.92)),
            ));
        });
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy observer parameters require On<T> by value"
)]
fn on_compass_click(click: On<PointerClick>, mut cam_state: ResMut<GlobeCameraState>) {
    if click.button == PointerButton::Primary {
        cam_state.start_north_reset();
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy observer parameters require On<T> by value"
)]
fn on_compass_over(
    over: On<PointerOver>,
    mut style_query: Query<(&mut BackgroundColor, &mut BorderColor), With<CompassButton>>,
) {
    if let Ok((mut bg, mut border)) = style_query.get_mut(over.entity) {
        *bg = BackgroundColor(COMPASS_BG_HOVER);
        *border = BorderColor::all(COMPASS_BORDER_HOVER);
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy observer parameters require On<T> by value"
)]
fn on_compass_out(
    out: On<PointerOut>,
    mut style_query: Query<(&mut BackgroundColor, &mut BorderColor), With<CompassButton>>,
) {
    if let Ok((mut bg, mut border)) = style_query.get_mut(out.entity) {
        *bg = BackgroundColor(COMPASS_BG_IDLE);
        *border = BorderColor::all(COMPASS_BORDER_IDLE);
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy observer parameters require On<T> by value"
)]
fn on_compass_press(
    press: On<PointerPress>,
    mut style_query: Query<(&mut BackgroundColor, &mut BorderColor), With<CompassButton>>,
) {
    if press.button == PointerButton::Primary
        && let Ok((mut bg, mut border)) = style_query.get_mut(press.entity)
    {
        *bg = BackgroundColor(COMPASS_BG_PRESS);
        *border = BorderColor::all(COMPASS_BORDER_PRESS);
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy observer parameters require On<T> by value"
)]
fn on_compass_release(
    release: On<PointerRelease>,
    mut style_query: Query<(&mut BackgroundColor, &mut BorderColor), With<CompassButton>>,
) {
    if release.button == PointerButton::Primary
        && let Ok((mut bg, mut border)) = style_query.get_mut(release.entity)
    {
        *bg = BackgroundColor(COMPASS_BG_HOVER);
        *border = BorderColor::all(COMPASS_BORDER_HOVER);
    }
}

/// Rotates the Compass dial toward live geographic North and updates the heading readout.
#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy ECS system parameters require Res<T> by value"
)]
pub fn update_compass_ui(
    cam_state: Res<GlobeCameraState>,
    mut dial_query: Query<&mut UiTransform, With<CompassDial>>,
    mut heading_query: Query<&mut Text, With<CompassHeadingText>>,
) {
    let north_angle = cam_state.compass_north_angle_rad();
    #[expect(
        clippy::cast_possible_truncation,
        reason = "UI compass dial rotation is a visual f32 angle"
    )]
    let dial_rot = Rot2::radians((-north_angle) as f32);
    for mut tf in &mut dial_query {
        tf.rotation = dial_rot;
    }

    let heading_deg = north_angle.to_degrees().rem_euclid(360.0).round();
    let display_deg = if heading_deg >= 360.0 {
        0.0
    } else {
        heading_deg
    };
    for mut text in &mut heading_query {
        text.0 = format!("{display_deg:.0}°");
    }
}
