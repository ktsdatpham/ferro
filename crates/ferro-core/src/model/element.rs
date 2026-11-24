use super::bim::{Floor, FloorType, Material, Opening, Wall, WallType};
use super::{Category, ElementId, ParamValue, Uid};
use crate::math::{BoundingBox3, Length, Point3};
use crate::topo::Body;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A named elevation (D-16). `is_building_storey` marks real storeys
/// (`IfcBuildingStorey`), as in Q-E2.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Level {
    pub elevation: Length,
    pub is_building_storey: bool,
}

/// Free-form geometry edited directly (D-12, D-38). Geometry is stored in
/// model coordinates (relative to the Internal Origin).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Form {
    pub body: Body,
}

/// Where an element's geometry comes from (ADR 0004 §1). The geometric
/// variants of [`ElementBody`] are the source: `Wall`, `Floor` and `Opening`
/// carry the parameters their geometry is generated from; `Form` is geometry
/// edited directly, whatever the element's category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeometrySource {
    Parametric,
    Direct,
}

/// Category-specific data of an element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ElementBody {
    Level(Level),
    Form(Form),
    /// Boxed: a material with assets is much larger than other bodies.
    Material(Box<Material>),
    WallType(WallType),
    FloorType(FloorType),
    Wall(Wall),
    Floor(Floor),
    Opening(Opening),
    /// A construction stage (category `Phases`, ADR 0006).
    Phase(super::phase::Phase),
    /// A view: camera, cut, style, hidden elements, phase (category
    /// `Views`, D-33).
    View(super::view::View),
    /// An option set (category `DesignOptions`, ADR 0006).
    OptionSet(super::design_option::OptionSet),
    /// A design option: a layer of changes over the shared model (category
    /// `DesignOptions`, hosted by its option set; ADR 0006).
    DesignOption(super::design_option::DesignOption),
}

impl ElementBody {
    /// Stable name of the body kind, as written in files (the `kind` tag):
    /// `level`, `form`, `material`, `wall_type`, `floor_type`, `wall`,
    /// `floor`, `opening`, `phase`.
    pub fn kind(&self) -> &'static str {
        match self {
            ElementBody::Level(_) => "level",
            ElementBody::Form(_) => "form",
            ElementBody::Material(_) => "material",
            ElementBody::WallType(_) => "wall_type",
            ElementBody::FloorType(_) => "floor_type",
            ElementBody::Wall(_) => "wall",
            ElementBody::Floor(_) => "floor",
            ElementBody::Opening(_) => "opening",
            ElementBody::Phase(_) => "phase",
            ElementBody::View(_) => "view",
            ElementBody::OptionSet(_) => "option_set",
            ElementBody::DesignOption(_) => "design_option",
        }
    }
}

/// Anything in the document with identity (Foundation Spec §3.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Element {
    pub id: ElementId,
    pub uid: Uid,
    pub category: Category,
    pub name: Option<String>,
    /// `FamilyType` (reserved until families land in milestone 2).
    pub type_id: Option<ElementId>,
    /// Home level (D-16, Q-E2).
    pub level_id: Option<ElementId>,
    /// Hosting element (doors in walls; reserved).
    pub host_id: Option<ElementId>,
    /// Design option membership (D-32, ADR 0006): always `None` in the
    /// shared model, whose options are layers of changes; in a document
    /// resolved for a combination, the option whose layer added the element
    /// (a derived index, `model::design_option::apply_layers`).
    pub design_option: Option<ElementId>,
    /// Phase the element is built in (Revit: *Phase Created*; ADR 0006).
    /// `None` = existing in every phase (`model::phase::phase_status`).
    pub phase_created: Option<ElementId>,
    /// Phase the element is demolished in (Revit: *Phase Demolished*); the
    /// same phase as `phase_created` or a later one. `None` = never.
    pub phase_demolished: Option<ElementId>,
    pub params: BTreeMap<String, ParamValue>,
    pub body: ElementBody,
    /// A pinned element cannot be moved or deleted (ADR 0008, Revit
    /// `Element.Pinned`); every other edit stays allowed. See
    /// [`Element::pin_moved`]. Omitted from files when `false`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub pinned: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// `true` when `after` is `before` moved as a whole: same number of points
/// (paired by index), at least one moved by more than `VERTEX_TOL`, and
/// every distance to three anchor points kept within `VERTEX_TOL` (a
/// translation, rotation or mirror). A reshape (some points move, others
/// stay, or distances change) is not a move.
fn rigid_motion(before: &[Point3], after: &[Point3]) -> bool {
    use crate::math::tol::VERTEX_TOL;
    if before.len() != after.len() || before.is_empty() {
        return false;
    }
    let moved = before
        .iter()
        .zip(after)
        .any(|(p, q)| p.distance_to(*q) > VERTEX_TOL);
    if !moved {
        return false;
    }
    let a = before[0];
    let far = |key: &dyn Fn(Point3) -> f64| {
        (0..before.len())
            .max_by(|i, j| key(before[*i]).total_cmp(&key(before[*j])))
            .unwrap_or(0)
    };
    let i1 = far(&|p| p.distance_to(a));
    let b = before[i1];
    let i2 = far(&|p| (p - a).cross(b - a).length());
    let anchors = [0, i1, i2];
    before.iter().zip(after).all(|(p, q)| {
        anchors
            .iter()
            .all(|&j| (p.distance_to(before[j]) - q.distance_to(after[j])).abs() <= VERTEX_TOL)
    })
}

/// Points that place a wall's curve: ends, and the middle of an arc.
fn curve_points(c: &crate::geom::Curve) -> Vec<Point3> {
    match c {
        crate::geom::Curve::Line(_) => vec![c.start(), c.end()],
        crate::geom::Curve::Arc(_) => vec![c.start(), c.point_at(0.5), c.end()],
    }
}

/// A floor's outline and holes as points (plan, z = 0).
fn outline_points(f: &Floor) -> Vec<Point3> {
    f.outer
        .iter()
        .chain(f.holes.iter().flatten())
        .map(|p| Point3::new(p.x, p.y, 0.0))
        .collect()
}

/// A body's vertices in id order, or `None` when the two bodies do not
/// have the same vertex ids (the topology changed: a reshape).
fn paired_vertices(a: &Body, b: &Body) -> Option<(Vec<Point3>, Vec<Point3>)> {
    let pa: Vec<_> = a.vertices().collect();
    let pb: Vec<_> = b.vertices().collect();
    let same_ids = pa.len() == pb.len() && pa.iter().zip(&pb).all(|(x, y)| x.0 == y.0);
    same_ids.then(|| {
        (
            pa.into_iter().map(|x| x.1).collect(),
            pb.into_iter().map(|x| x.1).collect(),
        )
    })
}

impl Element {
    pub fn as_form(&self) -> Option<&Form> {
        match &self.body {
            ElementBody::Form(f) => Some(f),
            _ => None,
        }
    }
    pub fn as_level(&self) -> Option<&Level> {
        match &self.body {
            ElementBody::Level(l) => Some(l),
            _ => None,
        }
    }
    /// Model geometry of the element, if it has any (Forms, walls, floors,
    /// openings).
    pub fn geometry(&self) -> Option<&Body> {
        match &self.body {
            ElementBody::Form(f) => Some(&f.body),
            ElementBody::Wall(w) => Some(&w.geometry),
            ElementBody::Floor(f) => Some(&f.geometry),
            ElementBody::Opening(o) => Some(&o.geometry),
            _ => None,
        }
    }
    /// Source of the element's geometry; `None` when it has none (levels,
    /// types, materials).
    pub fn geometry_source(&self) -> Option<GeometrySource> {
        match &self.body {
            ElementBody::Form(_) => Some(GeometrySource::Direct),
            ElementBody::Wall(_) | ElementBody::Floor(_) | ElementBody::Opening(_) => {
                Some(GeometrySource::Parametric)
            }
            _ => None,
        }
    }
    /// `true` for type and material definitions (not placed in the model).
    pub fn is_definition(&self) -> bool {
        matches!(
            self.body,
            ElementBody::Material(_) | ElementBody::WallType(_) | ElementBody::FloorType(_)
        )
    }
    pub fn as_wall(&self) -> Option<&Wall> {
        match &self.body {
            ElementBody::Wall(w) => Some(w),
            _ => None,
        }
    }
    pub fn param(&self, key: &str) -> Option<&ParamValue> {
        self.params.get(key)
    }
    pub fn bounding_box(&self) -> Option<BoundingBox3> {
        self.geometry().and_then(|b| b.bounding_box())
    }
    /// `true` when `after` is this element moved (ADR 0008 §2, as Revit):
    /// a level's elevation changed; a wall's location curve changed (moved,
    /// rotated or an end dragged); a door or window slid along its wall or
    /// put in another; a Form's body or a floor's outline moved as a whole
    /// (translation, rotation, mirror; see `rigid_motion`). Everything else
    /// is not a move and stays allowed on a pinned element: parameters,
    /// type, level and offsets, name, phase, category, editing a Form's
    /// shape (push / pull, moving some of its vertices, scaling) or a
    /// floor's profile, and regenerated values. Changes below `VERTEX_TOL`
    /// are not moves.
    pub fn pin_moved(&self, after: &Element) -> bool {
        use crate::math::tol::VERTEX_TOL;
        match (&self.body, &after.body) {
            (ElementBody::Level(a), ElementBody::Level(b)) => {
                (a.elevation.mm() - b.elevation.mm()).abs() > VERTEX_TOL
            }
            (ElementBody::Wall(a), ElementBody::Wall(b)) => {
                let (pa, pb) = (curve_points(&a.curve), curve_points(&b.curve));
                pa.len() != pb.len()
                    || pa
                        .iter()
                        .zip(&pb)
                        .any(|(p, q)| p.distance_to(*q) > VERTEX_TOL)
            }
            (ElementBody::Opening(a), ElementBody::Opening(b)) => {
                self.host_id != after.host_id
                    || (a.position.mm() - b.position.mm()).abs() > VERTEX_TOL
            }
            (ElementBody::Floor(a), ElementBody::Floor(b)) => {
                rigid_motion(&outline_points(a), &outline_points(b))
            }
            (ElementBody::Form(a), ElementBody::Form(b)) => {
                paired_vertices(&a.body, &b.body).is_some_and(|(pa, pb)| rigid_motion(&pa, &pb))
            }
            _ => false,
        }
    }
    /// Display label: name, else category and id.
    pub fn label(&self) -> String {
        self.name
            .clone()
            .unwrap_or_else(|| format!("{} {}", self.category, self.id))
    }
}

#[cfg(test)]
mod tests {
    use super::rigid_motion;
    use crate::math::tol::VERTEX_TOL;
    use crate::math::{Angle, Point3, Transform, UnitVec3, Vec3};

    fn square() -> Vec<Point3> {
        [(0.0, 0.0), (1000.0, 0.0), (1000.0, 500.0), (0.0, 500.0)]
            .iter()
            .map(|(x, y)| Point3::new(*x, *y, 0.0))
            .collect()
    }

    #[test]
    fn rigid_motions_are_moves_and_reshapes_are_not() {
        let a = square();
        let moved = |t: &Transform| a.iter().map(|p| t.apply(*p)).collect::<Vec<_>>();
        let t = Transform::translation(Vec3::new(10.0, -5.0, 3.0));
        assert!(rigid_motion(&a, &moved(&t)));
        let r = Transform::rotation(
            Point3::new(500.0, 250.0, 0.0),
            UnitVec3::Z,
            Angle::from_deg(30.0),
        );
        assert!(rigid_motion(&a, &moved(&r)));
        // Mirror (x -> -x) keeps every distance: a move too.
        let m: Vec<Point3> = a.iter().map(|p| Point3::new(-p.x, p.y, p.z)).collect();
        assert!(rigid_motion(&a, &m));
        // One corner dragged: a reshape.
        let mut s = a.clone();
        s[2] = Point3::new(1200.0, 500.0, 0.0);
        assert!(!rigid_motion(&a, &s));
        // Scaled: a reshape.
        let sc: Vec<Point3> = a.iter().map(|p| Point3::new(p.x * 2.0, p.y, p.z)).collect();
        assert!(!rigid_motion(&a, &sc));
    }

    #[test]
    fn degenerate_and_tolerance_edges() {
        let a = square();
        assert!(!rigid_motion(&a, &a), "unchanged");
        assert!(!rigid_motion(&[], &[]));
        assert!(!rigid_motion(&a, &a[..3]), "different point counts");
        let p = [Point3::new(1.0, 2.0, 3.0)];
        assert!(rigid_motion(&p, &[Point3::new(5.0, 2.0, 3.0)]), "one point");
        let nudge = |d: f64| {
            a.iter()
                .map(|p| Point3::new(p.x + d, p.y, p.z))
                .collect::<Vec<_>>()
        };
        assert!(
            !rigid_motion(&a, &nudge(VERTEX_TOL * 0.5)),
            "within tolerance"
        );
        assert!(rigid_motion(&a, &nudge(VERTEX_TOL * 2.0)));
    }
}
