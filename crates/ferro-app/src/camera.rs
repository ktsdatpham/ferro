//! Orbit camera (Z up). Rendering is camera-relative (D-05): GPU positions are
//! `f32` offsets from `origin`, picking and snapping stay in `f64`.

use ferro_core::{BoundingBox3, Point3, UnitVec3, Vec3};
use ferro_snap::{Projector, Ray};
use glam::{Mat4, Vec3 as G3, Vec4};

#[derive(Debug, Clone)]
pub struct OrbitCamera {
    pub target: Point3,
    /// Radians from +X, counter-clockwise seen from above.
    pub yaw: f64,
    /// Radians above the horizon.
    pub pitch: f64,
    /// Distance from target (mm).
    pub distance: f64,
    pub fov_y: f64,
    /// Orthographic (parallel) projection.
    pub parallel: bool,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        OrbitCamera {
            target: Point3::new(2000.0, 2000.0, 0.0),
            yaw: (-60f64).to_radians(),
            pitch: 30f64.to_radians(),
            distance: 20_000.0,
            fov_y: 45f64.to_radians(),
            parallel: false,
        }
    }
}

impl OrbitCamera {
    pub fn eye(&self) -> Point3 {
        let (cp, sp) = (self.pitch.cos(), self.pitch.sin());
        self.target + Vec3::new(cp * self.yaw.cos(), cp * self.yaw.sin(), sp) * self.distance
    }

    pub fn orbit(&mut self, dx: f64, dy: f64) {
        self.yaw -= dx * 0.008;
        self.pitch = (self.pitch + dy * 0.008).clamp(-1.55, 1.55);
    }

    /// Pan by screen pixels.
    pub fn pan(&mut self, dx: f64, dy: f64, viewport_h: f64) {
        let world_per_px = 2.0 * self.distance * (self.fov_y * 0.5).tan() / viewport_h.max(1.0);
        let fwd = (self.target - self.eye())
            .normalize()
            .map(|u| u.as_vec())
            .unwrap_or(Vec3::Y);
        let right = fwd
            .cross(Vec3::Z)
            .normalize()
            .map(|u| u.as_vec())
            .unwrap_or(Vec3::X);
        let up = right.cross(fwd);
        self.target = self.target - right * (dx * world_per_px) + up * (dy * world_per_px);
    }

    /// Zoom by a factor (> 1 = closer), moving towards `focus` when given.
    pub fn zoom(&mut self, factor: f64, focus: Option<Point3>) {
        let factor = factor.clamp(0.2, 5.0);
        let new_d = (self.distance / factor).clamp(50.0, 5_000_000.0);
        if let Some(f) = focus {
            let k = 1.0 - new_d / self.distance;
            self.target = self.target + (f - self.target) * k;
        }
        self.distance = new_d;
    }

    pub fn zoom_extents(&mut self, bb: Option<BoundingBox3>) {
        let Some(bb) = bb else {
            *self = OrbitCamera {
                yaw: self.yaw,
                pitch: self.pitch,
                ..Default::default()
            };
            return;
        };
        self.target = bb.center();
        let r = (bb.size().length() * 0.5).max(1000.0);
        self.distance = r / (self.fov_y * 0.5).sin() * 1.1;
    }

    pub fn set_view(&mut self, yaw_deg: f64, pitch_deg: f64) {
        self.yaw = yaw_deg.to_radians();
        self.pitch = pitch_deg.to_radians();
    }

    /// View-projection relative to `origin`, for a viewport aspect.
    pub fn view_proj(&self, origin: Point3, aspect: f32) -> Mat4 {
        let rel = |p: Point3| {
            let v = p - origin;
            G3::new(v.x as f32, v.y as f32, v.z as f32)
        };
        let up = if self.pitch.abs() > 1.54 {
            // Looking straight down/up: use yaw direction as up.
            G3::new(-(self.yaw.cos() as f32), -(self.yaw.sin() as f32), 0.0)
        } else {
            G3::Z
        };
        let view = look_at_rh(rel(self.eye()), rel(self.target), up);
        let near = (self.distance * 0.002).max(5.0) as f32;
        let far = (self.distance * 200.0 + 2_000_000.0) as f32;
        let proj = if self.parallel {
            // Same framing as the perspective view at the target distance.
            let h = (self.distance * (self.fov_y * 0.5).tan()) as f32;
            orthographic_rh(h * aspect.max(0.01), h, -far, far)
        } else {
            perspective_rh(self.fov_y as f32, aspect.max(0.01), near, far)
        };
        proj * view
    }
}

/// Right-handed view matrix.
fn look_at_rh(eye: G3, center: G3, up: G3) -> Mat4 {
    let f = (center - eye).normalize();
    let s = f.cross(up).normalize();
    let u = s.cross(f);
    Mat4::from_cols(
        Vec4::new(s.x, u.x, -f.x, 0.0),
        Vec4::new(s.y, u.y, -f.y, 0.0),
        Vec4::new(s.z, u.z, -f.z, 0.0),
        Vec4::new(-s.dot(eye), -u.dot(eye), f.dot(eye), 1.0),
    )
}

/// Right-handed perspective with depth in [0, 1] (wgpu convention).
fn perspective_rh(fov_y: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / (fov_y * 0.5).tan();
    Mat4::from_cols(
        Vec4::new(f / aspect, 0.0, 0.0, 0.0),
        Vec4::new(0.0, f, 0.0, 0.0),
        Vec4::new(0.0, 0.0, far / (near - far), -1.0),
        Vec4::new(0.0, 0.0, near * far / (near - far), 0.0),
    )
}

/// Right-handed orthographic projection with depth in [0, 1]; `hw`, `hh` are
/// half extents.
fn orthographic_rh(hw: f32, hh: f32, near: f32, far: f32) -> Mat4 {
    Mat4::from_cols(
        Vec4::new(1.0 / hw, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 1.0 / hh, 0.0, 0.0),
        Vec4::new(0.0, 0.0, 1.0 / (near - far), 0.0),
        Vec4::new(0.0, 0.0, near / (near - far), 1.0),
    )
}

/// Camera snapshot for one frame: projection and picking in a viewport rect.
#[derive(Debug, Clone, Copy)]
pub struct FrameCamera {
    pub view_proj: Mat4,
    pub inv_view_proj: Mat4,
    pub origin: Point3,
    /// Viewport rect in egui points: min x, min y, width, height.
    pub rect: [f32; 4],
}

impl FrameCamera {
    pub fn new(cam: &OrbitCamera, origin: Point3, rect: [f32; 4]) -> FrameCamera {
        let vp = cam.view_proj(origin, rect[2] / rect[3].max(1.0));
        FrameCamera {
            view_proj: vp,
            inv_view_proj: vp.inverse(),
            origin,
            rect,
        }
    }

    /// Picking ray through a screen position (egui points).
    pub fn ray(&self, pos: [f32; 2]) -> Ray {
        let nx = (pos[0] - self.rect[0]) / self.rect[2] * 2.0 - 1.0;
        let ny = 1.0 - (pos[1] - self.rect[1]) / self.rect[3] * 2.0;
        let un = |z: f32| {
            let p = self.inv_view_proj * Vec4::new(nx, ny, z, 1.0);
            let p = p / p.w;
            self.origin + Vec3::new(p.x as f64, p.y as f64, p.z as f64)
        };
        let a = un(0.0);
        let b = un(1.0);
        Ray {
            origin: a,
            dir: (b - a).normalize().unwrap_or(-UnitVec3::Z),
        }
    }
}

impl Projector for FrameCamera {
    fn project(&self, p: Point3) -> Option<[f32; 2]> {
        let v = p - self.origin;
        let c = self.view_proj * Vec4::new(v.x as f32, v.y as f32, v.z as f32, 1.0);
        if c.w <= 1e-6 {
            return None;
        }
        let (nx, ny) = (c.x / c.w, c.y / c.w);
        Some([
            self.rect[0] + (nx + 1.0) * 0.5 * self.rect[2],
            self.rect[1] + (1.0 - ny) * 0.5 * self.rect[3],
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ray_through_projected_point_hits_it() {
        let cam = OrbitCamera::default();
        let fc = FrameCamera::new(
            &cam,
            Point3::new(1000.0, 1000.0, 0.0),
            [0.0, 0.0, 800.0, 600.0],
        );
        let p = Point3::new(3000.0, 4000.0, 500.0);
        let s = fc.project(p).unwrap();
        let r = fc.ray(s);
        let t = r.param_of(p);
        assert!(r.at(t).distance_to(p) < 5.0, "{}", r.at(t).distance_to(p));
    }

    #[test]
    fn parallel_projection_picks_too() {
        let cam = OrbitCamera {
            parallel: true,
            ..Default::default()
        };
        let fc = FrameCamera::new(
            &cam,
            Point3::new(1000.0, 1000.0, 0.0),
            [0.0, 0.0, 800.0, 600.0],
        );
        let p = Point3::new(3000.0, 4000.0, 500.0);
        let s = fc.project(p).unwrap();
        let r = fc.ray(s);
        let t = r.param_of(p);
        assert!(r.at(t).distance_to(p) < 5.0);
        // All rays are parallel.
        let r2 = fc.ray([10.0, 10.0]);
        assert!(r.dir.dot(r2.dir) > 1.0 - 1e-9);
    }

    #[test]
    fn zoom_towards_focus_moves_target() {
        let mut cam = OrbitCamera::default();
        let before = cam.target;
        cam.zoom(2.0, Some(Point3::new(10_000.0, 0.0, 0.0)));
        assert!(cam.distance < 20_000.0);
        assert!(cam.target.x > before.x);
    }
}
