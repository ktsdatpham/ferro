//! Views as elements (D-33, M5-22): `View3D`, `PlanView` and `SectionView`.
//!
//! A [`View`] is the body of an element of category `Views` (Revit: `View`,
//! `OST_Views`). It carries what the app used to keep as screen state: the
//! camera, the cut (section plane / section box, or the plan's view range),
//! the display style, elements and tags hidden in the view, and the view's
//! phase with its phase filter (ADR 0006 §3). Views are saved, undoable and
//! usable by agents like any other element.
//!
//! References to other elements (`phase`, `hidden_elements`,
//! `design_options`) are element ids in memory and uids in files and in the
//! change format (`model::change::visit_element_refs`). A plan view's level
//! is the element's `level_id` (Revit: the plan's associated level).
//!
//! All lengths are millimetres, angles radians (D-01, D-02).

use super::phase::PhaseStatus;
use super::{Category, Document, Element, ElementBody, ElementId};
use crate::math::{tol, Angle, BoundingBox3, Length, Point3, UnitVec3, Vec3};
use serde::{Deserialize, Serialize};

/// Default cut height of a plan view above its level (Revit: 1200 mm).
pub const DEFAULT_CUT_HEIGHT_MM: f64 = 1200.0;
/// Default depth of a section view behind its plane (mm).
pub const DEFAULT_SECTION_DEPTH_MM: f64 = 10_000.0;
/// Default vertical field of view (45°).
pub const DEFAULT_FOV_Y_RAD: f64 = std::f64::consts::FRAC_PI_4;

/// A plane cutting a view: a point on it and the normal of the side that is
/// cut away (not drawn). Used by 3D views (optional cut) and section views
/// (the view's plane).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SectionPlane {
    /// A point on the plane (mm, model coordinates).
    pub point: Point3,
    /// Normal pointing to the side that is cut away.
    pub normal: UnitVec3,
}

/// How the camera projects (Revit: perspective / orthographic 3D view).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Projection {
    #[default]
    Perspective,
    /// Orthographic: framed like the perspective view at the target distance.
    Parallel,
}

/// Camera of a view: where it stands, what it looks at, which way is up and
/// how it projects. Lengths in mm, `fov_y` in radians.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ViewCamera {
    pub eye: Point3,
    pub target: Point3,
    pub up: UnitVec3,
    #[serde(default)]
    pub projection: Projection,
    /// Vertical field of view, radians in (0, π). Parallel views use it to
    /// frame the target distance.
    #[serde(default = "default_fov")]
    pub fov_y: Angle,
}

fn default_fov() -> Angle {
    Angle::from_rad(DEFAULT_FOV_Y_RAD)
}

impl ViewCamera {
    /// Iso 3D camera: looking at the origin from the south-west-ish side
    /// (yaw −60°, pitch 30°, 20 m away), perspective.
    pub fn default_3d() -> ViewCamera {
        ViewCamera {
            eye: Point3::new(8_660.254, -15_000.0, 10_000.0),
            target: Point3::ORIGIN,
            up: UnitVec3::Z,
            projection: Projection::Perspective,
            fov_y: default_fov(),
        }
    }

    /// Top-down parallel camera over `(x, y)` looking at height `z`.
    pub fn plan(x: f64, y: f64, z: f64) -> ViewCamera {
        ViewCamera {
            eye: Point3::new(x, y, z + 20_000.0),
            target: Point3::new(x, y, z),
            up: UnitVec3::Y,
            projection: Projection::Parallel,
            fov_y: default_fov(),
        }
    }

    /// Parallel camera looking at a section plane from the side cut away.
    pub fn section(plane: &SectionPlane) -> ViewCamera {
        let up = if plane.normal.is_parallel(UnitVec3::Z) {
            UnitVec3::Y
        } else {
            UnitVec3::Z
        };
        ViewCamera {
            eye: plane.point + plane.normal * 20_000.0,
            target: plane.point,
            up,
            projection: Projection::Parallel,
            fov_y: default_fov(),
        }
    }

    /// Check the camera: finite coordinates, eye and target further apart
    /// than `VERTEX_TOL`, `up` not along the view direction, `fov_y` in
    /// (0, π). The error says what is wrong.
    pub fn validate(&self) -> Result<(), String> {
        let finite = |p: Point3| p.x.is_finite() && p.y.is_finite() && p.z.is_finite();
        if !finite(self.eye) || !finite(self.target) {
            return Err("camera coordinates must be finite numbers".into());
        }
        let dir: Vec3 = self.target - self.eye;
        if tol::is_zero_len(dir.length()) {
            return Err("the camera eye and target are the same point".into());
        }
        let Some(d) = dir.normalize() else {
            return Err("the camera eye and target are the same point".into());
        };
        if d.as_vec().cross(self.up.as_vec()).length() <= tol::ANGLE_TOL {
            return Err("the camera up direction is along the view direction".into());
        }
        let f = self.fov_y.rad();
        if !f.is_finite() || f <= tol::ANGLE_TOL || f >= std::f64::consts::PI - tol::ANGLE_TOL {
            return Err("the field of view must be between 0° and 180°".into());
        }
        Ok(())
    }
}

/// Vertical range of a plan view, as offsets from its level (Revit: *View
/// Range*): top of the range, cut plane, bottom and view depth, in mm.
/// Valid when `view_depth ≤ bottom ≤ cut_height ≤ top`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ViewRange {
    pub top: Length,
    pub cut_height: Length,
    pub bottom: Length,
    pub view_depth: Length,
}

impl Default for ViewRange {
    /// Revit's default: top 2300, cut 1200, bottom 0, depth 0 mm.
    fn default() -> Self {
        ViewRange {
            top: Length::from_mm(2300.0),
            cut_height: Length::from_mm(DEFAULT_CUT_HEIGHT_MM),
            bottom: Length::ZERO,
            view_depth: Length::ZERO,
        }
    }
}

impl ViewRange {
    /// Check the order `view_depth ≤ bottom ≤ cut_height ≤ top` (equal
    /// within `VERTEX_TOL` is allowed) and that every value is finite.
    pub fn validate(&self) -> Result<(), String> {
        let v = [
            self.view_depth.mm(),
            self.bottom.mm(),
            self.cut_height.mm(),
            self.top.mm(),
        ];
        if v.iter().any(|x| !x.is_finite()) {
            return Err("view range values must be finite numbers".into());
        }
        if v.windows(2).any(|w| w[1] < w[0] - tol::VERTEX_TOL) {
            return Err("the view range needs view depth ≤ bottom ≤ cut height ≤ top".into());
        }
        Ok(())
    }
}

/// What kind of view, with its kind-specific cut (Revit: `ViewType`
/// ThreeD / FloorPlan / Section).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ViewType {
    /// A 3D view (Revit `View3D`), optionally cut by a plane and / or
    /// limited to a section box.
    #[serde(rename = "view_3d")]
    View3D {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        section_plane: Option<SectionPlane>,
        /// Revit *Section Box*: only what is inside is drawn (stored; the
        /// app does not draw it yet).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        section_box: Option<BoundingBox3>,
    },
    /// A plan of the element's level (Revit `ViewPlan`), cut at
    /// `view_range.cut_height` above it.
    PlanView { view_range: ViewRange },
    /// A section (Revit `ViewSection`): cut by `section_plane`, showing
    /// `depth` mm beyond it (Revit *Far Clip Offset*).
    SectionView {
        section_plane: SectionPlane,
        depth: Length,
    },
}

impl ViewType {
    /// Stable snake_case name: `view_3d`, `plan_view`, `section_view`.
    pub fn as_str(&self) -> &'static str {
        match self {
            ViewType::View3D { .. } => "view_3d",
            ViewType::PlanView { .. } => "plan_view",
            ViewType::SectionView { .. } => "section_view",
        }
    }
}

/// How a view draws faces and edges (Revit: `DisplayStyle`). The app's
/// existing styles; presentation styles (D-49 `RenderStyle`) come later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayStyle {
    /// Coloured faces with edges.
    #[default]
    Shaded,
    /// Translucent faces, edges on top.
    XRay,
    /// Edges only.
    Wireframe,
    /// White faces, black edges (Revit *Hidden Line*).
    HiddenLine,
}

impl DisplayStyle {
    /// All styles, in menu order.
    pub const ALL: [DisplayStyle; 4] = [
        DisplayStyle::Shaded,
        DisplayStyle::XRay,
        DisplayStyle::Wireframe,
        DisplayStyle::HiddenLine,
    ];

    /// Label for menus.
    pub fn label(self) -> &'static str {
        match self {
            DisplayStyle::Shaded => "Shaded",
            DisplayStyle::XRay => "X-ray",
            DisplayStyle::Wireframe => "Wireframe",
            DisplayStyle::HiddenLine => "Hidden line",
        }
    }
}

/// How an element of some [`PhaseStatus`] is drawn in a view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseGraphics {
    /// Like an unphased element (Revit: *By Category*).
    Normal,
    /// With the status graphics: existing grey, demolished / temporary red
    /// and dashed (Revit: *Overridden*).
    Overridden,
    /// Not drawn (Revit: *Not Displayed*).
    Hidden,
}

/// Which statuses a view shows and how (Revit: *Phase Filter*). Future and
/// past elements are never shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseFilter {
    /// Everything standing or demolished in the phase; existing,
    /// demolished and temporary with their status graphics.
    #[default]
    ShowAll,
    /// The state at the end of the phase: existing and new, drawn normally.
    ShowComplete,
    /// Only what is new in the phase.
    ShowNew,
    /// Existing and demolished, with status graphics.
    ShowPreviousAndDemo,
    /// Existing (status graphics) and new.
    ShowPreviousAndNew,
    /// Demolished / temporary (status graphics) and new.
    ShowDemoAndNew,
    /// Every shown status drawn normally.
    NoOverrides,
}

impl PhaseFilter {
    /// All filters, in menu order.
    pub const ALL: [PhaseFilter; 7] = [
        PhaseFilter::ShowAll,
        PhaseFilter::ShowComplete,
        PhaseFilter::ShowNew,
        PhaseFilter::ShowPreviousAndDemo,
        PhaseFilter::ShowPreviousAndNew,
        PhaseFilter::ShowDemoAndNew,
        PhaseFilter::NoOverrides,
    ];

    /// `true` for the default filter (left out of files).
    pub fn is_default(&self) -> bool {
        *self == PhaseFilter::ShowAll
    }

    /// Label for menus (Revit names).
    pub fn label(self) -> &'static str {
        match self {
            PhaseFilter::ShowAll => "Show All",
            PhaseFilter::ShowComplete => "Show Complete",
            PhaseFilter::ShowNew => "Show New",
            PhaseFilter::ShowPreviousAndDemo => "Show Previous + Demo",
            PhaseFilter::ShowPreviousAndNew => "Show Previous + New",
            PhaseFilter::ShowDemoAndNew => "Show Demo + New",
            PhaseFilter::NoOverrides => "No overrides",
        }
    }

    /// How an element of status `s` is drawn under this filter. Pure and
    /// total; `Future` and `Past` are always `Hidden`.
    pub fn graphics(self, s: PhaseStatus) -> PhaseGraphics {
        use PhaseFilter as F;
        use PhaseGraphics::{Hidden as H, Normal as N, Overridden as O};
        use PhaseStatus as S;
        match (s, self) {
            (S::Future | S::Past, _) => H,
            (_, F::NoOverrides) => N,
            (S::New, F::ShowPreviousAndDemo) => H,
            (S::New, _) => N,
            (S::Existing, F::ShowAll | F::ShowPreviousAndDemo | F::ShowPreviousAndNew) => O,
            (S::Existing, F::ShowComplete) => N,
            (S::Existing, F::ShowNew | F::ShowDemoAndNew) => H,
            (S::Demolished | S::Temporary, F::ShowAll | F::ShowDemoAndNew) => O,
            (S::Demolished, F::ShowPreviousAndDemo) => O,
            (S::Temporary, F::ShowPreviousAndDemo) => H,
            (S::Demolished | S::Temporary, F::ShowComplete | F::ShowNew) => H,
            (S::Demolished | S::Temporary, F::ShowPreviousAndNew) => H,
        }
    }
}

/// A view (D-33): body of an element of category `Views`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct View {
    pub view_type: ViewType,
    pub camera: ViewCamera,
    #[serde(default)]
    pub display_style: DisplayStyle,
    /// Elements hidden in this view (Revit: *Hide in View → Elements*),
    /// sorted, unique. Deleting an element removes it from every view in
    /// the same transaction.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_elements: Vec<ElementId>,
    /// Tags (`ferro.common.tags`) whose elements are not shown, sorted,
    /// unique (D-74 visibility by tag).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_tags: Vec<String>,
    /// The view's phase (a `Phases` element); `None` = the last phase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<ElementId>,
    #[serde(default, skip_serializing_if = "PhaseFilter::is_default")]
    pub phase_filter: PhaseFilter,
    /// Reserved for the design-option combination shown (one option per
    /// option set, ADR 0006 §Design options 3; M5-23.9). Always empty until
    /// design options land; already walked as references.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub design_options: Vec<ElementId>,
}

impl View {
    /// A view of type `view_type` with `camera`, default style (shaded for
    /// 3D, hidden line for plans and sections) and nothing hidden.
    pub fn new(view_type: ViewType, camera: ViewCamera) -> View {
        let display_style = match view_type {
            ViewType::View3D { .. } => DisplayStyle::Shaded,
            _ => DisplayStyle::HiddenLine,
        };
        View {
            view_type,
            camera,
            display_style,
            hidden_elements: Vec::new(),
            hidden_tags: Vec::new(),
            phase: None,
            phase_filter: PhaseFilter::ShowAll,
            design_options: Vec::new(),
        }
    }

    /// The plane cutting the view: a 3D view's optional plane, a section's
    /// plane, or for a plan the horizontal plane at `level_elevation_mm +
    /// cut_height` with the part above cut away.
    pub fn cut_plane(&self, level_elevation_mm: f64) -> Option<SectionPlane> {
        match &self.view_type {
            ViewType::View3D { section_plane, .. } => *section_plane,
            ViewType::SectionView { section_plane, .. } => Some(*section_plane),
            ViewType::PlanView { view_range } => Some(SectionPlane {
                point: Point3::new(0.0, 0.0, level_elevation_mm + view_range.cut_height.mm()),
                normal: UnitVec3::Z,
            }),
        }
    }

    /// `true` if `id` is hidden in this view by id (tags not included).
    pub fn hides(&self, id: ElementId) -> bool {
        self.hidden_elements.binary_search(&id).is_ok()
    }
}

/// Check a section box: finite corners and `min < max` by more than
/// `VERTEX_TOL` on every axis.
pub fn validate_section_box(b: &BoundingBox3) -> Result<(), String> {
    let (a, z) = (b.min, b.max);
    let all = [a.x, a.y, a.z, z.x, z.y, z.z];
    if all.iter().any(|v| !v.is_finite()) {
        return Err("section box corners must be finite numbers".into());
    }
    if z.x - a.x <= tol::VERTEX_TOL || z.y - a.y <= tol::VERTEX_TOL || z.z - a.z <= tol::VERTEX_TOL
    {
        return Err("the section box needs min < max on every axis".into());
    }
    Ok(())
}

/// Check a section plane's point is finite.
pub fn validate_section_plane(p: &SectionPlane) -> Result<(), String> {
    if [p.point.x, p.point.y, p.point.z]
        .iter()
        .any(|v| !v.is_finite())
    {
        return Err("the section plane point must be finite".into());
    }
    Ok(())
}

impl Element {
    /// The view data, if this element is a view.
    pub fn as_view(&self) -> Option<&View> {
        match &self.body {
            ElementBody::View(v) => Some(v),
            _ => None,
        }
    }
}

impl Document {
    /// The project's views, sorted by name (then id).
    pub fn views(&self) -> Vec<&Element> {
        let mut v: Vec<&Element> = self
            .by_category(Category::Views)
            .filter(|e| e.as_view().is_some())
            .collect();
        v.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        v
    }

    /// The plane cutting view element `view` ([`View::cut_plane`], with a
    /// plan's level elevation); `None` if it is not a view or has no cut.
    pub fn view_cut_plane(&self, view: &Element) -> Option<SectionPlane> {
        let v = view.as_view()?;
        v.cut_plane(super::generate::level_elevation(self, view.level_id))
    }
}

#[cfg(test)]
mod tests {
    use super::PhaseGraphics::*;
    use super::*;

    #[test]
    fn default_cameras_are_valid() {
        ViewCamera::default_3d().validate().unwrap();
        ViewCamera::plan(1000.0, 2000.0, 3300.0).validate().unwrap();
        for n in [UnitVec3::X, UnitVec3::Z, -UnitVec3::Y] {
            let p = SectionPlane {
                point: Point3::new(1.0, 2.0, 3.0),
                normal: n,
            };
            ViewCamera::section(&p).validate().unwrap();
        }
    }

    #[test]
    fn degenerate_cameras_are_refused() {
        let mut c = ViewCamera::default_3d();
        c.target = c.eye;
        assert!(c.validate().unwrap_err().contains("same point"));
        let mut c = ViewCamera::default_3d();
        c.up = (c.target - c.eye).normalize().unwrap();
        assert!(c.validate().unwrap_err().contains("up"));
        let mut c = ViewCamera::default_3d();
        c.fov_y = Angle::from_rad(0.0);
        assert!(c.validate().is_err());
        c.fov_y = Angle::from_rad(std::f64::consts::PI);
        assert!(c.validate().is_err());
        let mut c = ViewCamera::default_3d();
        c.eye.x = f64::NAN;
        assert!(c.validate().is_err());
    }

    #[test]
    fn camera_tolerance_edge() {
        // Eye and target just outside VERTEX_TOL apart: valid; inside: not.
        let mut c = ViewCamera::plan(0.0, 0.0, 0.0);
        c.eye = Point3::new(0.0, 0.0, tol::VERTEX_TOL * 2.0);
        c.validate().unwrap();
        c.eye = Point3::new(0.0, 0.0, tol::VERTEX_TOL * 0.5);
        assert!(c.validate().is_err());
    }

    #[test]
    fn view_range_order_with_tolerance() {
        ViewRange::default().validate().unwrap();
        let mut r = ViewRange::default();
        r.cut_height = Length::from_mm(r.top.mm() + tol::VERTEX_TOL * 0.5);
        r.validate().unwrap();
        r.cut_height = Length::from_mm(r.top.mm() + 1.0);
        assert!(r.validate().is_err());
        let mut r = ViewRange::default();
        r.view_depth = Length::from_mm(f64::INFINITY);
        assert!(r.validate().is_err());
    }

    #[test]
    fn section_box_needs_volume() {
        let b = |d: f64| BoundingBox3 {
            min: Point3::ORIGIN,
            max: Point3::new(1000.0, 1000.0, d),
        };
        validate_section_box(&b(1000.0)).unwrap();
        validate_section_box(&b(tol::VERTEX_TOL * 2.0)).unwrap();
        assert!(validate_section_box(&b(tol::VERTEX_TOL * 0.5)).is_err());
        assert!(validate_section_box(&b(-5.0)).is_err());
    }

    #[test]
    fn phase_filters_follow_revit() {
        use PhaseStatus::*;
        let g = |f: PhaseFilter| -> Vec<PhaseGraphics> {
            [Existing, New, Demolished, Temporary]
                .iter()
                .map(|s| f.graphics(*s))
                .collect()
        };
        assert_eq!(
            g(PhaseFilter::ShowAll),
            [Overridden, Normal, Overridden, Overridden]
        );
        assert_eq!(
            g(PhaseFilter::ShowComplete),
            [Normal, Normal, Hidden, Hidden]
        );
        assert_eq!(g(PhaseFilter::ShowNew), [Hidden, Normal, Hidden, Hidden]);
        assert_eq!(
            g(PhaseFilter::ShowPreviousAndDemo),
            [Overridden, Hidden, Overridden, Hidden]
        );
        assert_eq!(
            g(PhaseFilter::ShowPreviousAndNew),
            [Overridden, Normal, Hidden, Hidden]
        );
        assert_eq!(
            g(PhaseFilter::ShowDemoAndNew),
            [Hidden, Normal, Overridden, Overridden]
        );
        assert_eq!(g(PhaseFilter::NoOverrides), [Normal; 4]);
        for f in PhaseFilter::ALL {
            assert_eq!(f.graphics(Future), Hidden);
            assert_eq!(f.graphics(Past), Hidden);
        }
    }

    #[test]
    fn plan_cut_plane_follows_the_level_and_cut_height() {
        let v = View::new(
            ViewType::PlanView {
                view_range: ViewRange::default(),
            },
            ViewCamera::plan(0.0, 0.0, 0.0),
        );
        let p = v.cut_plane(3300.0).unwrap();
        assert!(tol::eq_len(p.point.z, 4500.0));
        assert_eq!(p.normal, UnitVec3::Z);
        assert_eq!(v.display_style, DisplayStyle::HiddenLine);
    }

    #[test]
    fn body_json_shape_is_stable() {
        let v = View::new(
            ViewType::View3D {
                section_plane: None,
                section_box: None,
            },
            ViewCamera::default_3d(),
        );
        let j = serde_json::to_value(&v).unwrap();
        assert_eq!(j["view_type"]["type"], "view_3d");
        assert_eq!(j["display_style"], "shaded");
        assert!(j.get("hidden_elements").is_none(), "empty lists left out");
        assert!(j.get("phase_filter").is_none(), "default filter left out");
        let back: View = serde_json::from_value(j).unwrap();
        assert_eq!(back, v);
    }
}
