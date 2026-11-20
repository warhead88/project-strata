use bevy::{prelude::*, window::PrimaryWindow};

use crate::camera::GlobeCameraState;

#[derive(Component)]
pub(crate) struct ScaleLength;

#[derive(Component)]
pub(crate) struct ScaleText;

pub(crate) fn spawn(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(16.0),
                left: Val::Px(16.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.04, 0.06, 0.09, 0.78)),
        ))
        .with_children(|children| {
            children.spawn((
                ScaleText,
                Text::new(""),
                TextFont {
                    font_size: FontSize::Px(13.0),
                    ..default()
                },
            ));
            children.spawn((
                ScaleLength,
                Node {
                    height: Val::Px(5.0),
                    border: UiRect {
                        bottom: Val::Px(2.0),
                        left: Val::Px(2.0),
                        right: Val::Px(2.0),
                        ..default()
                    },
                    ..default()
                },
                BorderColor::all(Color::WHITE),
            ));
        });
}

#[expect(
    clippy::needless_pass_by_value,
    clippy::cast_possible_truncation,
    reason = "Bevy system parameters and pixel widths bounded by the scale bar maximum"
)]
pub(crate) fn update(
    camera: Res<GlobeCameraState>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut bar: Single<&mut Node, With<ScaleLength>>,
    mut label: Single<&mut Text, With<ScaleText>>,
) {
    let (eye, _, _) = camera.compute_pose_f64();
    let target = camera.focus_ecef();
    let horizontal_m_per_px =
        2.0 * eye.distance_to(target) * (camera.fov_y_rad * 0.5).tan() / f64::from(window.height());
    if !horizontal_m_per_px.is_finite() || horizontal_m_per_px <= 0.0 {
        return;
    }
    let (length_m, width_px) = choose_bar(horizontal_m_per_px);
    let width = Val::Px(width_px as f32);
    if bar.width != width {
        bar.width = width;
    }
    let text = if length_m >= 1000.0 {
        format!("{:.0} km", length_m / 1000.0)
    } else {
        format!("{length_m:.0} m")
    };
    if label.0 != text {
        label.0 = text;
    }
}

fn choose_bar(m_per_px: f64) -> (f64, f64) {
    const TARGET_WIDTH_PX: f64 = 90.0;
    let target = TARGET_WIDTH_PX * m_per_px;
    let base = 10.0_f64.powf(target.log10().floor());
    let length = if 5.0 * base <= target {
        5.0 * base
    } else if 2.0 * base <= target {
        2.0 * base
    } else {
        base
    };
    (length, length / m_per_px)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regional_scale_bar_spans_twenty_kilometers() {
        let (length, px) = choose_bar(285.0);
        assert!((length - 20_000.0).abs() < f64::EPSILON);
        assert!((px - 70.175_438_596_5).abs() < 1e-8);
    }
}
