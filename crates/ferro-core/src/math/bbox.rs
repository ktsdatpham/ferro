use super::{Point3, Vec3};
use serde::{Deserialize, Serialize};

/// Axis-aligned bounding box in model space (mm).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoundingBox3 {
    pub min: Point3,
    pub max: Point3,
}

impl BoundingBox3 {
    /// Smallest box containing all points; `None` for an empty input.
    pub fn from_points<I: IntoIterator<Item = Point3>>(points: I) -> Option<BoundingBox3> {
        let mut it = points.into_iter();
        let first = it.next()?;
        Some(it.fold(
            BoundingBox3 {
                min: first,
                max: first,
            },
            |b, p| b.including(p),
        ))
    }

    pub fn including(self, p: Point3) -> BoundingBox3 {
        BoundingBox3 {
            min: Point3::new(
                self.min.x.min(p.x),
                self.min.y.min(p.y),
                self.min.z.min(p.z),
            ),
            max: Point3::new(
                self.max.x.max(p.x),
                self.max.y.max(p.y),
                self.max.z.max(p.z),
            ),
        }
    }

    pub fn union(self, o: BoundingBox3) -> BoundingBox3 {
        self.including(o.min).including(o.max)
    }

    pub fn size(self) -> Vec3 {
        self.max - self.min
    }

    pub fn center(self) -> Point3 {
        Point3::midpoint(self.min, self.max)
    }

    /// Inclusive containment (boundary counts as inside).
    pub fn contains(self, p: Point3) -> bool {
        (self.min.x..=self.max.x).contains(&p.x)
            && (self.min.y..=self.max.y).contains(&p.y)
            && (self.min.z..=self.max.z).contains(&p.z)
    }

    pub fn intersects(self, o: BoundingBox3) -> bool {
        self.min.x <= o.max.x
            && o.min.x <= self.max.x
            && self.min.y <= o.max.y
            && o.min.y <= self.max.y
            && self.min.z <= o.max.z
            && o.min.z <= self.max.z
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_points_and_contains() {
        let b = BoundingBox3::from_points([
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(4000.0, -1000.0, 2800.0),
        ])
        .unwrap();
        assert!(b.contains(Point3::new(2000.0, -500.0, 1400.0)));
        assert!(b.contains(b.max));
        assert!(!b.contains(Point3::new(4001.0, 0.0, 0.0)));
    }

    #[test]
    fn empty_input_has_no_box() {
        assert!(BoundingBox3::from_points(std::iter::empty()).is_none());
    }

    #[test]
    fn touching_boxes_intersect() {
        let a = BoundingBox3::from_points([Point3::ORIGIN, Point3::new(1.0, 1.0, 1.0)]).unwrap();
        let b = BoundingBox3::from_points([Point3::new(1.0, 0.0, 0.0), Point3::new(2.0, 1.0, 1.0)])
            .unwrap();
        assert!(a.intersects(b));
    }
}
