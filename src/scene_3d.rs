use glam::{EulerRot, Mat4, Quat, Vec2, Vec3};
use mega_render::{Camera, GizmoAxis, GizmoMode, Projection, ViewAxis};
use mega_ui::Rect;

mod gizmo;
pub(crate) use gizmo::GizmoDrag;
mod render;
pub(crate) use render::{RenderStyle, SceneRenderer, SceneThumbnails};

pub(crate) const THUMB_TEXTURE: u32 = u32::MAX;
const THUMB_SIZE: u32 = 96;
const THUMB_COLUMNS: u32 = 16;

pub(crate) struct ThumbnailLayout {
    pub size: (u32, u32),
}

impl ThumbnailLayout {
    pub fn new(count: usize) -> Self {
        let count = count.max(1) as u32;
        Self {
            size: (
                count.min(THUMB_COLUMNS) * THUMB_SIZE,
                count.div_ceil(THUMB_COLUMNS) * THUMB_SIZE,
            ),
        }
    }

    fn cell(&self, index: usize) -> Rect {
        let index = index as u32;
        let x = (index % THUMB_COLUMNS) * THUMB_SIZE;
        let y = (index / THUMB_COLUMNS) * THUMB_SIZE;
        Rect::from_min_size(
            Vec2::new(x as f32, y as f32),
            Vec2::splat(THUMB_SIZE as f32),
        )
    }

    pub fn uv(&self, index: usize) -> ([f32; 2], [f32; 2]) {
        let cell = self.cell(index);
        let size = Vec2::new(self.size.0 as f32, self.size.1 as f32);
        (
            ((cell.min + 0.5) / size).to_array(),
            ((cell.max - 0.5) / size).to_array(),
        )
    }

    fn index_at(&self, uv: Vec2) -> usize {
        let tiles = uv * Vec2::new(self.size.0 as f32, self.size.1 as f32) / THUMB_SIZE as f32;
        tiles.y as usize * THUMB_COLUMNS as usize + tiles.x as usize
    }

    fn view_rect(&self, index: usize, aspect: f32) -> Rect {
        let cell = self.cell(index);
        fit_frame(
            Rect {
                min: cell.min + 1.0,
                max: cell.max - 1.0,
            },
            aspect,
        )
    }
}

fn fit_frame(bounds: Rect, aspect: f32) -> Rect {
    let size = bounds.max - bounds.min;
    let extent = Vec2::new(size.x.min(size.y * aspect), size.y.min(size.x / aspect)).max(Vec2::ONE);
    Rect::from_min_size((bounds.min + bounds.max - extent) * 0.5, extent)
}

#[derive(Clone, Copy, PartialEq, Hash)]
pub(crate) enum Primitive {
    Cube,
    Sphere,
    Plane,
    Cylinder,
    Cone,
}

impl Primitive {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Cube => "Cube",
            Self::Sphere => "Sphere",
            Self::Plane => "Plane",
            Self::Cylinder => "Cylinder",
            Self::Cone => "Cone",
        }
    }
}

#[derive(Clone, PartialEq)]
pub(crate) struct Object {
    pub id: usize,
    pub name: String,
    pub visible: bool,
    pub primitive: Primitive,
    pub position: Vec3,
    pub rotation: Vec3,
    pub scale: Vec3,
}

impl Object {
    pub fn orientation(&self) -> Quat {
        let degrees = self.rotation * (std::f32::consts::PI / 180.0);
        Quat::from_euler(EulerRot::XYZ, degrees.x, degrees.y, degrees.z)
    }

    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.orientation(), self.position)
    }
}

#[derive(Clone)]
pub(crate) struct Objects {
    pub items: Vec<Object>,
    pub selected: Option<usize>,
}

impl Objects {
    pub fn selected(&self) -> Option<&Object> {
        self.selected.and_then(|index| self.items.get(index))
    }
}

pub(crate) struct OrbitCamera {
    target: Vec3,
    yaw: f32,
    pitch: f32,
    distance: f32,
    pub fov_y: f32,
    pub projection: Projection,
    snap: Option<ViewSnap>,
    ortho_from_axis: bool,
}

struct ViewSnap {
    from: Vec2,
    to: Vec2,
    progress: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            target: Vec3::new(0.0, 0.5, 0.0),
            yaw: 0.7,
            pitch: 0.48,
            distance: 7.0,
            fov_y: std::f32::consts::FRAC_PI_4,
            projection: Projection::Perspective,
            snap: None,
            ortho_from_axis: false,
        }
    }
}

impl OrbitCamera {
    fn direction(&self) -> Vec3 {
        Vec3::new(
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            self.yaw.cos() * self.pitch.cos(),
        )
    }

    pub fn eye(&self) -> Vec3 {
        self.target + self.direction() * self.distance
    }

    fn right(&self) -> Vec3 {
        Vec3::new(-self.yaw.cos(), 0.0, self.yaw.sin())
    }

    pub fn render_camera(&self) -> Camera {
        let mut camera = Camera::look_at(self.eye(), self.target);
        camera.up = (-self.direction()).cross(self.right());
        camera.fov_y = self.fov_y;
        camera.near = 0.03;
        camera.far = 4096.0;
        camera.projection = self.projection;
        camera.sync_ortho_from_distance();
        camera
    }

    pub fn view_projection(&self, size: Vec2) -> Mat4 {
        self.render_camera().view_proj(size.x / size.y)
    }

    pub fn snap_to_axis(&mut self, axis: ViewAxis) {
        let direction = axis.dir();
        let yaw = if direction.y.abs() == 1.0 {
            0.0
        } else {
            direction.x.atan2(direction.z)
        };
        let pitch = direction.y.asin();
        let delta_yaw = (yaw - self.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        self.snap = Some(ViewSnap {
            from: Vec2::new(self.yaw, self.pitch),
            to: Vec2::new(self.yaw + delta_yaw, pitch),
            progress: 0.0,
        });
        self.projection = Projection::Orthographic;
        self.ortho_from_axis = true;
    }

    pub fn tick_snap(&mut self, dt: f32) -> bool {
        let Some(snap) = &mut self.snap else {
            return false;
        };
        snap.progress = (snap.progress + dt / 0.22).min(1.0);
        let t = snap.progress;
        let angles = snap.from.lerp(snap.to, t * t * (3.0 - 2.0 * t));
        self.yaw = angles.x;
        self.pitch = angles.y;
        if t == 1.0 {
            self.snap = None;
        }
        true
    }

    pub fn toggle_projection(&mut self) {
        self.ortho_from_axis = false;
        self.projection = match self.projection {
            Projection::Perspective => Projection::Orthographic,
            Projection::Orthographic => Projection::Perspective,
        };
    }

    pub fn orbit(&mut self, delta: Vec2) {
        if delta == Vec2::ZERO {
            return;
        }
        self.snap = None;
        if self.ortho_from_axis {
            self.projection = Projection::Perspective;
            self.ortho_from_axis = false;
        }
        self.yaw = (self.yaw + delta.x * 0.008).rem_euclid(std::f32::consts::TAU);
        self.pitch = (self.pitch + delta.y * 0.008).clamp(-1.54, 1.54);
    }

    pub fn pan(&mut self, delta: Vec2, height: f32) {
        let right = self.right();
        let up = (-self.direction()).cross(right);
        let units_per_pixel = self.distance * (self.fov_y * 0.5).tan() * 2.0 / height;
        self.target += (up * delta.y - right * delta.x) * units_per_pixel;
    }

    pub fn zoom(&mut self, scroll: f32) {
        self.distance = (self.distance * (-scroll * 0.004).exp()).clamp(0.12, 3500.0);
    }

    pub fn focus(&mut self, object: &Object, aspect: f32) {
        let radius = object.scale.length() * 0.5;
        let half_angle = ((self.fov_y * 0.5).tan() * aspect.min(1.0)).atan();
        self.target = object.position;
        self.distance = (radius * 1.2 / half_angle.sin()).clamp(0.12, 3500.0);
    }

    pub fn project(&self, rect: Rect, position: Vec3) -> Option<Vec2> {
        let clip = self.view_projection(rect.max - rect.min) * position.extend(1.0);
        if clip.w <= 0.0 || !(0.0..=clip.w).contains(&clip.z) {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(rect.min + (ndc.truncate() * Vec2::new(0.5, -0.5) + 0.5) * (rect.max - rect.min))
    }

    pub fn ray(&self, rect: Rect, point: Vec2) -> (Vec3, Vec3) {
        let ndc = (point - rect.min) / (rect.max - rect.min) * Vec2::new(2.0, -2.0)
            + Vec2::new(-1.0, 1.0);
        let near = self
            .view_projection(rect.max - rect.min)
            .inverse()
            .project_point3(ndc.extend(0.0));
        let direction = match self.projection {
            Projection::Perspective => (near - self.eye()).normalize(),
            Projection::Orthographic => -self.direction(),
        };
        (near, direction)
    }
}

pub(crate) fn axis_parameter(ray: (Vec3, Vec3), origin: Vec3, axis: Vec3) -> Option<f32> {
    let (eye, direction) = ray;
    let cosine = direction.dot(axis);
    let divisor = 1.0 - cosine * cosine;
    let offset = eye - origin;
    (divisor > 0.002).then(|| (offset.dot(axis) - cosine * offset.dot(direction)) / divisor)
}

pub(crate) struct SceneDocument {
    pub id: usize,
    pub name: String,
    pub camera: OrbitCamera,
    pub view_aspect: f32,
    pub objects: Objects,
    pub drag: Option<GizmoDrag>,
    pub gizmo_mode: GizmoMode,
    pub gizmo_hover: Option<GizmoAxis>,
    pub orbiting: bool,
    pub panning: bool,
    next_object: usize,
    undo: Vec<Objects>,
    redo: Vec<Objects>,
    edit: Option<Objects>,
}

impl SceneDocument {
    fn new(id: usize, name: String) -> Self {
        Self {
            id,
            name,
            camera: OrbitCamera::default(),
            view_aspect: 1.0,
            objects: Objects {
                items: Vec::new(),
                selected: None,
            },
            drag: None,
            gizmo_mode: GizmoMode::Translate,
            gizmo_hover: None,
            orbiting: false,
            panning: false,
            next_object: 1,
            undo: Vec::new(),
            redo: Vec::new(),
            edit: None,
        }
    }

    pub(crate) fn layer_render_stamp(&self, size: (u32, u32)) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut state = std::collections::hash_map::DefaultHasher::new();
        size.hash(&mut state);
        self.view_aspect.to_bits().hash(&mut state);
        self.camera
            .view_projection(Vec2::ONE)
            .to_cols_array()
            .map(f32::to_bits)
            .hash(&mut state);
        for object in self.objects.items.iter().filter(|object| object.visible) {
            object.primitive.hash(&mut state);
            object
                .matrix()
                .to_cols_array()
                .map(f32::to_bits)
                .hash(&mut state);
        }
        state.finish()
    }

    pub fn remember(&mut self, before: Objects) {
        const HISTORY_LIMIT: usize = 64;
        if self.undo.len() == HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.undo.push(before);
        self.redo.clear();
    }

    pub fn begin_edit(&mut self, before: Objects) {
        if self.edit.is_none() {
            self.edit = Some(before);
        }
    }

    pub fn finish_edit(&mut self, cancel: bool) {
        self.drag = None;
        if let Some(before) = self.edit.take() {
            if cancel {
                self.objects = before;
            } else if before.items != self.objects.items {
                self.remember(before);
            }
        }
    }

    pub fn undo(&mut self) {
        self.finish_edit(false);
        if let Some(previous) = self.undo.pop() {
            self.redo
                .push(std::mem::replace(&mut self.objects, previous));
        }
    }

    pub fn redo(&mut self) {
        self.finish_edit(false);
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.objects, next));
        }
    }

    pub fn add(&mut self, primitive: Primitive) {
        self.finish_edit(false);
        self.remember(self.objects.clone());
        let number = self.next_object;
        self.next_object += 1;
        self.objects.selected = Some(self.objects.items.len());
        self.objects.items.push(Object {
            id: number,
            name: format!("{} {number}", primitive.name()),
            visible: true,
            primitive,
            position: Vec3::new(
                (number - 1) as f32 * 1.25,
                if primitive == Primitive::Plane {
                    0.0
                } else {
                    0.5
                },
                0.0,
            ),
            rotation: Vec3::ZERO,
            scale: Vec3::ONE,
        });
    }

    pub fn clone_object(&mut self, index: usize) {
        self.finish_edit(false);
        if let Some(mut copy) = self.objects.items.get(index).cloned() {
            self.remember(self.objects.clone());
            copy.id = self.next_object;
            self.next_object += 1;
            copy.name.push_str(" copy");
            copy.position.x += 1.25 * copy.scale.x;
            self.objects.selected = Some(self.objects.items.len());
            self.objects.items.push(copy);
        }
    }

    pub fn delete(&mut self) {
        if let Some(index) = self.objects.selected {
            self.delete_object(index);
        }
    }

    pub fn delete_object(&mut self, index: usize) {
        self.finish_edit(false);
        if index >= self.objects.items.len() {
            return;
        }
        self.remember(self.objects.clone());
        self.objects.items.remove(index);
        self.objects.selected = self.objects.selected.and_then(|selected| {
            (selected != index).then(|| selected - usize::from(selected > index))
        });
    }

    pub fn toggle_object_visibility(&mut self, index: usize) {
        self.finish_edit(false);
        if index < self.objects.items.len() {
            self.remember(self.objects.clone());
            self.objects.items[index].visible = !self.objects.items[index].visible;
            self.gizmo_hover = None;
        }
    }

    pub fn rename_object(&mut self, id: usize, original: &str, name: String) -> bool {
        self.finish_edit(false);
        let Some(index) = self
            .objects
            .items
            .iter()
            .position(|object| object.id == id && object.name == original)
        else {
            return false;
        };
        if self.objects.items[index].name == name {
            return false;
        }
        self.remember(self.objects.clone());
        self.objects.items[index].name = name;
        true
    }

    pub fn pick(&self, ray: (Vec3, Vec3)) -> Option<usize> {
        self.objects
            .items
            .iter()
            .enumerate()
            .filter(|(_, object)| object.visible)
            .filter_map(|(index, object)| hit(object, ray).map(|distance| (index, distance)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(index, _)| index)
    }
}

fn hit(object: &Object, (origin, direction): (Vec3, Vec3)) -> Option<f32> {
    let inverse = object.matrix().inverse();
    let origin = inverse.transform_point3(origin);
    // Keep the direction unnormalized so distances remain comparable in world space.
    let direction = inverse.transform_vector3(direction);
    let (enter, exit) = match object.primitive {
        Primitive::Plane => {
            let t = hit_horizontal_plane(origin, direction, 0.0)?;
            let point = origin + direction * t;
            return (point.x.abs() <= 0.5 && point.z.abs() <= 0.5).then_some(t);
        }
        Primitive::Cylinder => return hit_round_solid(origin, direction, false),
        Primitive::Cone => return hit_round_solid(origin, direction, true),
        Primitive::Sphere => {
            let a = direction.length_squared();
            let b = origin.dot(direction);
            let discriminant = b * b - a * (origin.length_squared() - 0.25);
            if discriminant < 0.0 {
                return None;
            }
            let root = discriminant.sqrt();
            ((-b - root) / a, (-b + root) / a)
        }
        Primitive::Cube => {
            let mut enter = f32::NEG_INFINITY;
            let mut exit = f32::INFINITY;
            for (position, direction) in origin.to_array().into_iter().zip(direction.to_array()) {
                if direction.abs() < 1e-8 {
                    if position.abs() > 0.5 {
                        return None;
                    }
                } else {
                    let a = (-0.5 - position) / direction;
                    let b = (0.5 - position) / direction;
                    enter = enter.max(a.min(b));
                    exit = exit.min(a.max(b));
                }
            }
            (enter, exit)
        }
    };
    (exit >= enter && exit >= 0.0).then_some(if enter >= 0.0 { enter } else { exit })
}

fn hit_horizontal_plane(origin: Vec3, direction: Vec3, height: f32) -> Option<f32> {
    let t = (height - origin.y) / direction.y;
    (t.is_finite() && t >= 0.0).then_some(t)
}

fn hit_round_solid(origin: Vec3, direction: Vec3, cone: bool) -> Option<f32> {
    let radial = Vec2::new(origin.x, origin.z);
    let velocity = Vec2::new(direction.x, direction.z);
    let (radius, rate) = if cone {
        (0.25 - 0.5 * origin.y, -0.5 * direction.y)
    } else {
        (0.5, 0.0)
    };
    let a = velocity.length_squared() - rate * rate;
    let b = radial.dot(velocity) - radius * rate;
    let c = radial.length_squared() - radius * radius;
    let discriminant = b * b - a * c;
    let sides = if a.abs() < 1e-9 {
        [-c / (2.0 * b), f32::INFINITY]
    } else if discriminant >= 0.0 {
        let root = discriminant.sqrt();
        [(-b - root) / a, (-b + root) / a]
    } else {
        [f32::INFINITY; 2]
    };
    let cap = |height| {
        let t = hit_horizontal_plane(origin, direction, height)?;
        ((radial + velocity * t).length_squared() <= 0.25).then_some(t)
    };
    let side = sides
        .into_iter()
        .filter(|t| t.is_finite() && *t >= 0.0 && (origin.y + direction.y * t).abs() <= 0.5);
    side.chain(cap(-0.5))
        .chain((!cone).then(|| cap(0.5)).flatten())
        .min_by(f32::total_cmp)
}

#[derive(Default)]
pub(crate) struct SceneEditor {
    pub documents: Vec<SceneDocument>,
    pub active: Option<usize>,
    pub viewport: Option<Rect>,
    pub view_hud: mega_render::Hud,
    next_document: usize,
}

impl SceneEditor {
    pub fn create(&mut self) -> usize {
        self.next_document += 1;
        self.documents.push(SceneDocument::new(
            self.next_document,
            format!("Scene {}", self.next_document),
        ));
        self.documents.len() - 1
    }

    pub fn current(&self) -> Option<&SceneDocument> {
        self.documents.get(self.active?)
    }

    pub fn current_mut(&mut self) -> Option<&mut SceneDocument> {
        self.documents.get_mut(self.active?)
    }

    pub fn close_view(&mut self) -> bool {
        if let Some(doc) = self.current_mut() {
            doc.finish_edit(false);
            doc.orbiting = false;
            doc.panning = false;
            doc.gizmo_hover = None;
        }
        self.viewport = None;
        self.active.take().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_primitives_can_be_picked_and_hidden_objects_do_not_intercept_rays() {
        for primitive in [Primitive::Plane, Primitive::Cylinder, Primitive::Cone] {
            let mut scene = SceneDocument::new(0, "Picking".into());
            scene.add(primitive);
            let origin = scene.objects.items[0].position;
            let ray = (origin + Vec3::Y * 2.0, Vec3::NEG_Y);
            assert_eq!(scene.pick(ray), Some(0));
            assert_eq!(scene.pick((ray.0 + Vec3::X, ray.1)), None);
            let stamp = scene.layer_render_stamp((512, 512));
            scene.toggle_object_visibility(0);
            assert_eq!(scene.pick(ray), None);
            assert_ne!(scene.layer_render_stamp((512, 512)), stamp);
            scene.undo();
            assert_eq!(scene.pick(ray), Some(0));
            assert_eq!(scene.layer_render_stamp((512, 512)), stamp);
        }
    }

    #[test]
    fn field_of_view_changes_projection_and_invalidates_the_linked_render() {
        let mut scene = SceneDocument::new(0, "Camera".into());
        let rect = Rect::from_min_size(Vec2::ZERO, Vec2::splat(512.0));
        let before = scene.camera.project(rect, Vec3::X).unwrap();
        let stamp = scene.layer_render_stamp((512, 512));
        scene.camera.fov_y = 90.0_f32.to_radians();
        let after = scene.camera.project(rect, Vec3::X).unwrap();
        let center = (rect.min + rect.max) * 0.5;
        assert!(after.distance(center) < before.distance(center));
        assert_ne!(scene.layer_render_stamp((512, 512)), stamp);
    }

    #[test]
    fn thumbnail_preserves_the_camera_framing_and_maps_clipped_rows_to_their_scene() {
        let mut scene = SceneDocument::new(0, "Scene".into());
        scene.camera.pan(Vec2::new(22.0, -37.0), 400.0);
        scene.camera.toggle_projection();
        let viewport = Rect::from_min_size(Vec2::new(120.0, 50.0), Vec2::new(800.0, 400.0));
        let layout = ThumbnailLayout::new(20);
        let thumb = layout.view_rect(18, viewport.width() / viewport.height());
        let point = Vec3::Y;
        let frame = |rect: Rect| {
            (scene.camera.project(rect, point).unwrap() - rect.min) / (rect.max - rect.min)
        };
        assert!(frame(viewport).abs_diff_eq(frame(thumb), 1e-5));
        let (min, max) = layout.uv(18);
        let clipped_midpoint = Vec2::from_array(min).lerp(Vec2::from_array(max), 0.9);
        assert_eq!(layout.index_at(clipped_midpoint), 18);
    }
}
