use super::{PixelRect, Selection};
use crate::raster::{Raster, SparsePixels};
use glam::Vec2;

/// A regular UV mesh. Only positions change; UVs always refer to the stroke source.
pub struct DeformMesh {
    pub original: Vec<Vec2>,
    pub positions: Vec<Vec2>,
    pub cols: usize,
    pub rows: usize,
    pub width: u32,
    pub height: u32,
}

/// A pointer-down snapshot: membership, weights and tool settings remain fixed until release.
pub struct MeshGrab {
    vertices: Vec<(usize, Vec2, f32)>,
    smooth_targets: Vec<Vec2>,
    center: Vec2,
    radius: f32,
    strength: f32,
    mode: usize,
}

impl DeformMesh {
    pub fn new(width: u32, height: u32, divisions: usize) -> Self {
        let longest = width.max(height).max(1) as f32;
        let cols = ((width as f32 / longest * divisions as f32).round() as usize).max(1);
        let rows = ((height as f32 / longest * divisions as f32).round() as usize).max(1);
        let original: Vec<_> = (0..=rows)
            .flat_map(|y| {
                (0..=cols).map(move |x| {
                    Vec2::new(
                        x as f32 * width as f32 / cols as f32,
                        y as f32 * height as f32 / rows as f32,
                    )
                })
            })
            .collect();
        Self {
            positions: original.clone(),
            original,
            cols,
            rows,
            width,
            height,
        }
    }

    pub(crate) fn triangles(&self) -> impl Iterator<Item = [usize; 3]> + '_ {
        (0..self.rows).flat_map(move |y| {
            (0..self.cols).flat_map(move |x| {
                let a = y * (self.cols + 1) + x;
                let b = a + 1;
                let c = a + self.cols + 1;
                [[a, b, c + 1], [a, c + 1, c]]
            })
        })
    }

    pub fn grab(
        &self,
        center: Vec2,
        radius: f32,
        strength: f32,
        softness: f32,
        mode: usize,
    ) -> MeshGrab {
        let radius = radius.max(1.0);
        let mut vertices = Vec::new();
        for (i, &position) in self.positions.iter().enumerate() {
            let distance = position.distance(center) / radius;
            if distance >= 1.0 {
                continue;
            }
            let t = ((1.0 - distance) / softness.clamp(0.001, 1.0)).clamp(0.0, 1.0);
            vertices.push((i, position, t * t * (3.0 - 2.0 * t)));
        }
        // Freeze neighbour averages alongside the grab so smoothing remains independent
        // of pointer event frequency and never pulls toward the original UV grid.
        let smooth_targets = if mode == 3 {
            let stride = self.cols + 1;
            vertices
                .iter()
                .map(|&(i, _, _)| {
                    let x = i % stride;
                    let y = i / stride;
                    let mut sum = Vec2::ZERO;
                    let mut count = 0.0;
                    for neighbour in [
                        (x > 0).then(|| i - 1),
                        (x < self.cols).then(|| i + 1),
                        (y > 0).then(|| i - stride),
                        (y < self.rows).then(|| i + stride),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        sum += self.positions[neighbour];
                        count += 1.0;
                    }
                    sum / count
                })
                .collect()
        } else {
            Vec::new()
        };
        MeshGrab {
            smooth_targets,
            vertices,
            center,
            radius,
            strength,
            mode,
        }
    }

    /// Derive every update from the pointer-down snapshot, never from a moving brush footprint.
    pub fn drag(&mut self, grab: &MeshGrab, pointer: Vec2) -> bool {
        let delta = pointer - grab.center;
        let amount = delta.length() / grab.radius;
        let mut next = self.positions.clone();
        for (vertex, &(i, start, weight)) in grab.vertices.iter().enumerate() {
            let influence = weight * grab.strength;
            let offset = start - grab.center;
            next[i] = match grab.mode {
                1 => grab.center + offset * (1.0 + (amount + 0.08) * influence),
                2 => grab.center + offset / (1.0 + (amount + 0.08) * influence),
                3 => start.lerp(
                    grab.smooth_targets[vertex],
                    ((amount + 0.08) * influence).clamp(0.0, 1.0),
                ),
                4 => {
                    let angle = delta.x / grab.radius * influence;
                    let (sin, cos) = angle.sin_cos();
                    grab.center
                        + Vec2::new(
                            offset.x * cos - offset.y * sin,
                            offset.x * sin + offset.y * cos,
                        )
                }
                _ => start + delta * influence,
            };
        }
        // All modes operate directly on the captured mesh, including folds and edges.
        if next == self.positions {
            return false;
        }
        self.positions = next;
        true
    }

    #[cfg(test)]
    pub fn dab(
        &mut self,
        center: Vec2,
        delta: Vec2,
        radius: f32,
        strength: f32,
        softness: f32,
        mode: usize,
    ) -> bool {
        let grab = self.grab(center, radius, strength, softness, mode);
        self.drag(&grab, center + delta)
    }

    pub fn render(
        &self,
        source: &SparsePixels,
        selection: Option<&Selection>,
    ) -> (SparsePixels, Option<PixelRect>) {
        let mut min = Vec2::new(self.width as f32, self.height as f32);
        let mut max = Vec2::ZERO;
        for tri in self.triangles() {
            if tri.iter().any(|&i| self.positions[i] != self.original[i]) {
                for i in tri {
                    min = min.min(self.original[i]).min(self.positions[i]);
                    max = max.max(self.original[i]).max(self.positions[i]);
                }
            }
        }
        let bounds = PixelRect {
            min_x: min.x.floor().max(0.0) as u32,
            min_y: min.y.floor().max(0.0) as u32,
            max_x: (max.x.ceil() as u32).min(self.width),
            max_y: (max.y.ceil() as u32).min(self.height),
        };
        let mut out = source.clone();
        if bounds.is_empty() {
            return (out, None);
        }
        // A freely moved boundary may leave holes. Rebuild the affected area from
        // transparent, including unchanged triangles that can overlap a folded mesh.
        for y in bounds.min_y..bounds.max_y {
            for x in bounds.min_x..bounds.max_x {
                if selection.is_none_or(|s| s.contains(x, y)) {
                    out.write(self.width, x as i32, y as i32, [0; 4]);
                }
            }
        }
        for [a, b, c] in self.triangles() {
            let p = [self.positions[a], self.positions[b], self.positions[c]];
            let lo = p[0]
                .min(p[1])
                .min(p[2])
                .floor()
                .max(Vec2::new(bounds.min_x as f32, bounds.min_y as f32));
            let hi = p[0]
                .max(p[1])
                .max(p[2])
                .ceil()
                .min(Vec2::new(bounds.max_x as f32, bounds.max_y as f32));
            let area = cross(p[1] - p[0], p[2] - p[0]);
            if !area.is_finite() || area.abs() < 1e-7 {
                continue;
            }
            for y in lo.y as u32..hi.y as u32 {
                for x in lo.x as u32..hi.x as u32 {
                    if selection.is_some_and(|s| !s.contains(x, y)) {
                        continue;
                    }
                    let q = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - p[0];
                    let v = cross(q, p[2] - p[0]) / area;
                    let w = cross(p[1] - p[0], q) / area;
                    if v < -0.00001 || w < -0.00001 || v + w > 1.00001 {
                        continue;
                    }
                    let uv = self.original[a] * (1.0 - v - w)
                        + self.original[b] * v
                        + self.original[c] * w;
                    out.write(
                        self.width,
                        x as i32,
                        y as i32,
                        sample(source, self.width, self.height, uv),
                    );
                }
            }
        }
        (out, Some(bounds))
    }
}

fn cross(a: Vec2, b: Vec2) -> f32 {
    a.x * b.y - a.y * b.x
}

fn sample(source: &SparsePixels, width: u32, height: u32, uv: Vec2) -> [u8; 4] {
    let p = uv - Vec2::splat(0.5);
    let base = p.floor();
    let t = p - base;
    let mut sum = [0.0f32; 4];
    for (dx, dy, w) in [
        (0, 0, (1.0 - t.x) * (1.0 - t.y)),
        (1, 0, t.x * (1.0 - t.y)),
        (0, 1, (1.0 - t.x) * t.y),
        (1, 1, t.x * t.y),
    ] {
        let pixel = source.read(
            width,
            (base.x as i32 + dx).clamp(0, width as i32 - 1),
            (base.y as i32 + dy).clamp(0, height as i32 - 1),
        );
        let alpha = pixel[3] as f32 / 255.0;
        for c in 0..3 {
            sum[c] += pixel[c] as f32 * alpha * w;
        }
        sum[3] += alpha * w;
    }
    if sum[3] <= f32::EPSILON {
        return [0; 4];
    }
    [
        (sum[0] / sum[3]).round() as u8,
        (sum[1] / sum[3]).round() as u8,
        (sum[2] / sum[3]).round() as u8,
        (sum[3] * 255.0).round() as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_preserves_sparse_pixels_and_offset() {
        let mut source = SparsePixels::default();
        source.offset = (-3, 5);
        source.write(32, 10, 12, [12, 34, 56, 78]);
        let (out, dirty) = DeformMesh::new(32, 32, 8).render(&source, None);
        assert_eq!(source, out);
        assert!(dirty.is_none());
    }
    #[test]
    fn move_inflate_and_pinch_follow_their_expected_directions() {
        for mode in 0..3 {
            let mut mesh = DeformMesh::new(100, 100, 10);
            let center = Vec2::splat(50.0);
            assert!(mesh.dab(center, Vec2::new(5.0, 0.0), 40.0, 1.0, 1.0, mode));
            let i = 5 * 11 + 6;
            let dx = mesh.positions[i].x - mesh.original[i].x;
            assert!(if mode == 2 { dx < 0.0 } else { dx > 0.0 });
            assert_eq!(mesh.positions[0], Vec2::ZERO);
        }
    }
    #[test]
    fn selection_and_off_canvas_are_preserved() {
        let mut source = SparsePixels::default();
        for y in 0..32 {
            for x in 0..32 {
                source.write(32, x, y, [x as u8 * 7, y as u8 * 7, 100, 255]);
            }
        }
        source.write(32, -2, 5, [1, 2, 3, 255]);
        let mut mesh = DeformMesh::new(32, 32, 8);
        mesh.dab(Vec2::splat(16.0), Vec2::new(4.0, 0.0), 12.0, 1.0, 1.0, 0);
        let empty = Selection {
            width: 32,
            height: 32,
            mask: vec![0; 1024],
        };
        assert_eq!(mesh.render(&source, Some(&empty)).0, source);
        let out = mesh.render(&source, None).0;
        assert_ne!(out, source);
        assert_eq!(out.read(32, -2, 5), source.read(32, -2, 5));
        assert_eq!(out.read(32, 0, 0), source.read(32, 0, 0));
    }
    #[test]
    fn grab_keeps_membership_weights_and_is_independent_of_pointer_events() {
        for mode in 0..5 {
            let mut direct = DeformMesh::new(200, 200, 20);
            let mut stepped = DeformMesh::new(200, 200, 20);
            // Include an existing deformation so Normalize has something to smooth.
            direct.dab(Vec2::splat(100.0), Vec2::new(3.0, 0.0), 50.0, 0.5, 1.0, 0);
            stepped.positions = direct.positions.clone();
            let before = direct.positions.clone();
            let center = Vec2::splat(100.0);
            let a = direct.grab(center, 50.0, 0.5, 1.0, mode);
            let b = stepped.grab(center, 50.0, 0.5, 1.0, mode);
            let members: Vec<usize> = a.vertices.iter().map(|v| v.0).collect();
            direct.drag(&a, center + Vec2::new(15.0, 0.0));
            for x in [3.0, 6.0, 9.0, 12.0, 15.0] {
                stepped.drag(&b, center + Vec2::new(x, 0.0));
            }
            for i in 0..before.len() {
                assert!(
                    direct.positions[i].distance(stepped.positions[i]) < 0.0001,
                    "mode {mode}, vertex {i}"
                );
                if !members.contains(&i) {
                    assert_eq!(direct.positions[i], before[i]);
                }
            }
        }
    }

    #[test]
    fn move_uses_the_original_softness_weight_until_release() {
        let mut mesh = DeformMesh::new(200, 200, 20);
        let center = Vec2::splat(100.0);
        let grab = mesh.grab(center, 50.0, 1.0, 1.0, 0);
        let i = 10 * 21 + 7;
        let (_, start, weight) = *grab.vertices.iter().find(|v| v.0 == i).unwrap();
        mesh.drag(&grab, center + Vec2::new(10.0, 0.0));
        mesh.drag(&grab, center + Vec2::new(20.0, 0.0));
        assert!(mesh.positions[i].distance(start + Vec2::new(20.0, 0.0) * weight) < 0.0001);
        mesh.drag(&grab, center);
        assert_eq!(mesh.positions, mesh.original);
    }

    #[test]
    fn normalize_smooths_current_neighbours_and_rotate_preserves_pivot_distance() {
        let mut mesh = DeformMesh::new(200, 200, 20);
        let center = Vec2::splat(100.0);
        let i = 10 * 21 + 10;
        mesh.positions[i] += Vec2::new(2.0, 0.0);
        let grab = mesh.grab(center, 60.0, 1.0, 0.0, 3);
        mesh.drag(&grab, center + Vec2::new(60.0, 0.0));
        assert!(mesh.positions[i].distance(center) < 0.001);
        let grab = mesh.grab(center, 60.0, 0.5, 1.0, 4);
        let i = 10 * 21 + 11;
        let start = mesh.positions[i];
        mesh.drag(&grab, center + Vec2::new(15.0, 0.0));
        assert!(mesh.positions[i].y > start.y);
        assert!((mesh.positions[i].distance(center) - start.distance(center)).abs() < 0.001);
        assert_eq!(mesh.positions[0], mesh.original[0]);
    }
    #[test]
    fn normalize_preserves_translated_rotated_and_scaled_regular_mesh() {
        let mut mesh = DeformMesh::new(200, 200, 20);
        for p in &mut mesh.positions {
            let v = *p;
            *p = Vec2::new(v.x * 1.2 - v.y * 0.2, v.x * 0.2 + v.y * 1.2) + Vec2::new(30.0, 20.0);
        }
        let before = mesh.positions.clone();
        let center = mesh.positions[10 * 21 + 10];
        let grab = mesh.grab(center, 60.0, 1.0, 1.0, 3);
        mesh.drag(&grab, center + Vec2::new(60.0, 0.0));
        for (a, b) in mesh.positions.iter().zip(&before) {
            assert!(a.distance(*b) < 0.0001);
        }
        // An irregularity is smoothed at its current location, preserving the global transform.
        let i = 10 * 21 + 10;
        mesh.positions[i] += Vec2::new(2.0, 0.0);
        let grab = mesh.grab(center, 60.0, 1.0, 0.0, 3);
        mesh.drag(&grab, center + Vec2::new(60.0, 0.0));
        assert!(mesh.positions[i].distance(before[i]) < 0.001);
        assert!(mesh.positions[i].distance(mesh.original[i]) > 20.0);
    }
    #[test]
    fn move_follows_pointer_without_distance_or_fold_limits_and_grabs_edges() {
        let mut mesh = DeformMesh::new(100, 100, 10);
        let center = Vec2::splat(50.0);
        let grab = mesh.grab(center, 30.0, 1.0, 1.0, 0);
        let i = 5 * 11 + 5;
        for delta in [
            Vec2::new(20.0, 0.0),
            Vec2::new(500.0, -300.0),
            Vec2::new(-1000.0, 2000.0),
            Vec2::ZERO,
        ] {
            mesh.drag(&grab, center + delta);
            assert_eq!(mesh.positions[i], center + delta);
        }
        let grab = mesh.grab(Vec2::ZERO, 25.0, 1.0, 1.0, 0);
        mesh.drag(&grab, Vec2::new(-200.0, -100.0));
        assert_eq!(mesh.positions[0], Vec2::new(-200.0, -100.0));
    }

    #[test]
    fn applying_a_fold_or_collapsed_triangle_produces_finite_pixels() {
        let source = SparsePixels::from_rgba(&[100, 50, 20, 255].repeat(32 * 32), 32, 32);
        let mut mesh = DeformMesh::new(32, 32, 8);
        let grab = mesh.grab(Vec2::splat(16.0), 8.0, 1.0, 0.0, 0);
        mesh.drag(&grab, Vec2::new(80.0, -40.0));
        assert!(mesh.triangles().any(|[a, b, c]| cross(
            mesh.positions[b] - mesh.positions[a],
            mesh.positions[c] - mesh.positions[a]
        ) < 0.0));
        assert!(mesh.render(&source, None).1.is_some());
        mesh.positions.fill(Vec2::splat(16.0));
        let out = mesh.render(&source, None).0;
        assert!(out.to_rgba(32, 32).iter().all(|&v| v == 0));
    }
    #[test]
    fn unrestricted_modes_work_even_when_an_unrelated_part_of_mesh_is_folded() {
        for mode in 1..5 {
            let mut mesh = DeformMesh::new(200, 200, 20);
            let grab = mesh.grab(Vec2::splat(50.0), 30.0, 1.0, 0.0, 0);
            mesh.drag(&grab, Vec2::new(130.0, 50.0));
            assert!(mesh.triangles().any(|[a, b, c]| cross(
                mesh.positions[b] - mesh.positions[a],
                mesh.positions[c] - mesh.positions[a]
            ) < 0.0));
            let center = Vec2::splat(150.0);
            let probe = 15 * 21 + 16;
            if mode == 3 {
                mesh.positions[probe] += Vec2::new(4.0, 0.0);
            }
            let before = mesh.positions.clone();
            let grab = mesh.grab(center, 40.0, 1.0, 0.0, mode);
            assert!(
                mesh.drag(&grab, center + Vec2::new(100.0, 0.0)),
                "mode {mode} blocked by an unrelated fold"
            );
            assert_ne!(
                mesh.positions[probe], before[probe],
                "mode {mode} did not affect its captured vertex"
            );
            for i in 0..before.len() {
                if !grab.vertices.iter().any(|v| v.0 == i) {
                    assert_eq!(mesh.positions[i], before[i]);
                }
            }
        }
    }

    #[test]
    fn unrestricted_rotation_continues_through_multiple_turns_in_both_directions() {
        let mut mesh = DeformMesh::new(200, 200, 20);
        let center = Vec2::splat(100.0);
        let radius = 40.0;
        let grab = mesh.grab(center, radius, 1.0, 0.0, 4);
        let probe = 10 * 21 + 11;
        for turns in [0.25, 0.5, 1.25, 3.75, -2.25] {
            let angle = turns * std::f32::consts::TAU;
            mesh.drag(&grab, center + Vec2::new(angle * radius, 0.0));
            let expected = center + Vec2::new(angle.cos(), angle.sin()) * 10.0;
            assert!(
                mesh.positions[probe].distance(expected) < 0.001,
                "rotation stopped at {turns} turns"
            );
        }
    }

    #[test]
    fn unrestricted_modes_capture_boundary_vertices_without_neighbour_index_errors() {
        for mode in 1..5 {
            let mut mesh = DeformMesh::new(100, 100, 10);
            mesh.positions[0] += Vec2::new(2.0, 0.0);
            let grab = mesh.grab(Vec2::splat(5.0), 30.0, 1.0, 0.0, mode);
            assert!(
                grab.vertices.iter().any(|v| v.0 == 0),
                "mode {mode} cannot capture the corner"
            );
            let before = mesh.positions[0];
            mesh.drag(&grab, Vec2::new(35.0, 5.0));
            assert_ne!(mesh.positions[0], before);
            assert!(mesh.positions.iter().all(|p| p.is_finite()));
        }
    }

    #[test]
    fn radial_modes_remain_finite_and_respond_to_long_drags() {
        for mode in [1, 2] {
            let mut mesh = DeformMesh::new(200, 200, 20);
            let center = Vec2::splat(100.0);
            let grab = mesh.grab(center, 40.0, 1.0, 0.0, mode);
            let i = 10 * 21 + 11;
            mesh.drag(&grab, center + Vec2::new(100_000.0, 0.0));
            assert!(mesh.positions.iter().all(|p| p.is_finite()));
            let distance = mesh.positions[i].distance(center);
            assert!(if mode == 1 {
                distance > 20_000.0
            } else {
                distance < 0.01
            });
        }
    }
}
