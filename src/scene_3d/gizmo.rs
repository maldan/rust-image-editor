use glam::{EulerRot, Quat, Vec2, Vec3, vec3};
use mega_render::{
    DebugDraw, GizmoAxis, GizmoMode, GizmoOpts, GizmoRotateArc, gizmo_ring_basis, gizmo_screen_size,
};
use mega_ui::Rect;

use super::{Object, SceneDocument, axis_parameter};

const AXES: [(GizmoAxis, Vec3); 3] = [
    (GizmoAxis::X, Vec3::X),
    (GizmoAxis::Y, Vec3::Y),
    (GizmoAxis::Z, Vec3::Z),
];

pub(crate) struct GizmoDrag {
    pub axis: GizmoAxis,
    initial: Object,
    kind: DragKind,
}

enum DragKind {
    MoveAxis {
        direction: Vec3,
        start: f32,
    },
    MovePlane {
        normal: Vec3,
        start: Vec3,
    },
    Rotate {
        normal: Vec3,
        arc: GizmoRotateArc,
    },
    ScaleAxis {
        direction: Vec3,
        component: usize,
        start: f32,
        size: f32,
    },
    ScaleUniform {
        start: Vec2,
        pixels: f32,
    },
}

impl SceneDocument {
    pub fn gizmo_size(&self, rect: Rect, scale: f32, origin: Vec3) -> f32 {
        let camera = self.camera.render_camera();
        let distance = match camera.projection {
            mega_render::Projection::Perspective => camera.eye.distance(origin),
            mega_render::Projection::Orthographic => camera.eye.distance(camera.target),
        };
        gizmo_screen_size(distance, camera.fov_y, rect.height(), 85.0 * scale)
    }

    pub fn gizmo_geometry(&self, rect: Rect, scale: f32) -> DebugDraw {
        let mut debug = DebugDraw::default();
        if let Some(object) = self.objects.selected().filter(|object| object.visible) {
            debug.gizmo(
                object.position,
                self.gizmo_orientation(object),
                GizmoOpts {
                    mode: self.gizmo_mode,
                    size: self.gizmo_size(rect, scale, object.position),
                    highlight: self
                        .drag
                        .as_ref()
                        .map(|drag| drag.axis)
                        .or(self.gizmo_hover),
                    eye: Some(self.camera.eye()),
                    rotate_arc: self.drag.as_ref().and_then(|drag| match drag.kind {
                        DragKind::Rotate { arc, .. } => Some(arc),
                        _ => None,
                    }),
                    ..Default::default()
                },
            );
        }
        debug
    }

    fn gizmo_orientation(&self, object: &Object) -> Quat {
        if self.gizmo_mode == GizmoMode::Scale {
            object.orientation()
        } else {
            Quat::IDENTITY
        }
    }

    pub fn grab_gizmo(&self, rect: Rect, mouse: Vec2, scale: f32) -> Option<GizmoDrag> {
        let object = self.objects.selected().filter(|object| object.visible)?;
        let origin = object.position;
        let center = self.camera.project(rect, origin)?;
        let size = self.gizmo_size(rect, scale, origin);
        let ray = self.camera.ray(rect, mouse);
        let orientation = self.gizmo_orientation(object);
        let axes = AXES.map(|(_, direction)| orientation * direction);
        let grab = |axis, kind| GizmoDrag {
            axis,
            initial: object.clone(),
            kind,
        };
        let threshold = 7.0 * scale;
        match self.gizmo_mode {
            GizmoMode::Rotate => AXES
                .into_iter()
                .zip(axes)
                .filter_map(|((axis, _), normal)| {
                    let (u, v) = gizmo_ring_basis(axis, axes[0], axes[1], axes[2]);
                    let hit = plane_hit(ray, origin, normal)? - origin;
                    if hit.length_squared() < 1e-10 {
                        return None;
                    }
                    let rim = self.camera.project(rect, origin + hit.normalize() * size)?;
                    let distance = rim.distance(mouse);
                    let start = hit.dot(v).atan2(hit.dot(u));
                    let arc = GizmoRotateArc {
                        axis,
                        u,
                        v,
                        start,
                        current: start,
                    };
                    (distance <= threshold)
                        .then(|| (distance, grab(axis, DragKind::Rotate { normal, arc })))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, drag)| drag),
            GizmoMode::Translate | GizmoMode::Scale => {
                if self.gizmo_mode == GizmoMode::Scale && mouse.distance(center) <= 13.0 * scale {
                    return Some(grab(
                        GizmoAxis::Uniform,
                        DragKind::ScaleUniform {
                            start: mouse,
                            pixels: 85.0 * scale,
                        },
                    ));
                }
                let axis_hit = AXES
                    .into_iter()
                    .zip(axes)
                    .enumerate()
                    .filter_map(|(component, ((axis, _), direction))| {
                        let tip = self.camera.project(rect, origin + direction * size)?;
                        let screen_axis = tip - center;
                        if screen_axis.length() < 12.0 * scale {
                            return None;
                        }
                        let t = (mouse - center).dot(screen_axis) / screen_axis.length_squared();
                        let distance = mouse.distance(center + screen_axis * t.clamp(0.16, 1.03));
                        let start = axis_parameter(ray, origin, direction)?;
                        let kind = if self.gizmo_mode == GizmoMode::Scale {
                            DragKind::ScaleAxis {
                                direction,
                                component,
                                start,
                                size,
                            }
                        } else {
                            DragKind::MoveAxis { direction, start }
                        };
                        (distance <= threshold).then_some((distance, axis, kind))
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0));
                if let Some((_, axis, kind)) = axis_hit {
                    return Some(grab(axis, kind));
                }
                if self.gizmo_mode == GizmoMode::Translate {
                    return [
                        (GizmoAxis::Xy, axes[0], axes[1]),
                        (GizmoAxis::Yz, axes[1], axes[2]),
                        (GizmoAxis::Zx, axes[2], axes[0]),
                    ]
                    .into_iter()
                    .find_map(|(axis, u, v)| {
                        let normal = u.cross(v);
                        let start = plane_hit(ray, origin, normal)?;
                        let offset = (start - origin) / size;
                        let edge = 0.20..=0.44;
                        (edge.contains(&offset.dot(u)) && edge.contains(&offset.dot(v)))
                            .then(|| grab(axis, DragKind::MovePlane { normal, start }))
                    });
                }
                None
            }
        }
    }
}

impl GizmoDrag {
    pub fn update(&mut self, object: &mut Object, ray: (Vec3, Vec3), mouse: Vec2) {
        let origin = self.initial.position;
        match &mut self.kind {
            DragKind::MoveAxis { direction, start } => {
                if let Some(parameter) = axis_parameter(ray, origin, *direction) {
                    object.position = (origin + *direction * (parameter - *start))
                        .clamp(Vec3::splat(-10000.0), Vec3::splat(10000.0));
                }
            }
            DragKind::MovePlane { normal, start } => {
                if let Some(hit) = plane_hit(ray, origin, *normal) {
                    object.position = (origin + (hit - *start))
                        .clamp(Vec3::splat(-10000.0), Vec3::splat(10000.0));
                }
            }
            DragKind::Rotate { normal, arc } => {
                if let Some(hit) = plane_hit(ray, origin, *normal) {
                    let delta = hit - origin;
                    arc.current = delta.dot(arc.v).atan2(delta.dot(arc.u));
                    let angle = arc.current - arc.start;
                    if angle.abs() < 1e-5 {
                        object.rotation = self.initial.rotation;
                    } else {
                        let rotation =
                            Quat::from_axis_angle(*normal, angle) * self.initial.orientation();
                        let (x, y, z) = rotation.to_euler(EulerRot::XYZ);
                        object.rotation = vec3(x.to_degrees(), y.to_degrees(), z.to_degrees());
                    }
                }
            }
            DragKind::ScaleAxis {
                direction,
                component,
                start,
                size,
            } => {
                if let Some(parameter) = axis_parameter(ray, origin, *direction) {
                    let factor = (1.0 + (parameter - *start) / *size).max(0.0);
                    object.scale[*component] =
                        (self.initial.scale[*component] * factor).clamp(0.01, 1000.0);
                }
            }
            DragKind::ScaleUniform { start, pixels } => {
                let movement = mouse - *start;
                let factor = ((movement.x - movement.y) / *pixels).clamp(-8.0, 8.0).exp();
                object.scale =
                    (self.initial.scale * factor).clamp(Vec3::splat(0.01), Vec3::splat(1000.0));
            }
        }
    }
}

fn plane_hit((eye, dir): (Vec3, Vec3), origin: Vec3, normal: Vec3) -> Option<Vec3> {
    let denominator = dir.dot(normal);
    if denominator.abs() < 1e-4 {
        return None;
    }
    let t = (origin - eye).dot(normal) / denominator;
    (t >= 0.0).then(|| eye + dir * t)
}
