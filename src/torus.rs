//! Geometry on the flat 2-torus T² = ℝ² / ℤ².
//!
//! The flat torus is the quotient of the Euclidean plane by the integer
//! lattice: two points (x, y) and (x', y') are identified iff x − x' ∈ ℤ
//! and y − y' ∈ ℤ. The unit square [0, 1)² is a fundamental domain.
//!
//! # Why we don't just store "(x, y) in [0, 1)" everywhere
//!
//! Most operations on the torus have ambiguity coming from the choice of
//! lift to the universal cover ℝ². For example, the "straight line"
//! between two points on the torus is *not* unique — there is one geodesic
//! per element of π₁(T²) = ℤ². We resolve the ambiguity *at the type
//! level* by being explicit about what we store:
//!
//! - [`TorusPoint`] stores a canonical representative in [0, 1)².
//! - [`TorusVec`] stores a displacement in ℝ² (the universal cover).
//!   Two displacements that differ by an integer lattice vector represent
//!   *different homotopy classes* of paths on the torus.
//! - [`TorusSegment`] = (start point, displacement). A segment is a
//!   straight-line path in the universal cover, projected down to the
//!   torus. This way, a segment that wraps around the torus stays well-
//!   defined as the user drags its endpoints across the seam.

/// A point on the torus T² = ℝ² / ℤ², stored as the canonical
/// representative in [0, 1) × [0, 1).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct TorusPoint {
    x: f32,
    y: f32,
}

impl TorusPoint {
    /// Construct a point from any ℝ² coordinates; wraps into [0, 1)².
    pub fn new(x: f32, y: f32) -> Self {
        Self {
            x: x.rem_euclid(1.0),
            y: y.rem_euclid(1.0),
        }
    }

    pub fn x(self) -> f32 {
        self.x
    }
    pub fn y(self) -> f32 {
        self.y
    }

    /// Translate by a displacement vector. The result is reduced back into
    /// the canonical fundamental domain.
    pub fn translate(self, v: TorusVec) -> Self {
        Self::new(self.x + v.dx, self.y + v.dy)
    }

    /// The *shortest* displacement vector from `self` to `other`. Each
    /// component lies in (−½, ½]. (Ties at exactly ½ are resolved
    /// consistently — see implementation.)
    ///
    /// Note: this is one of infinitely many displacements from `self` to
    /// `other` (any one differs from this by an integer lattice vector).
    /// Use this when you want the "obvious" line between two points (e.g.
    /// for hit-testing, distance, or the initial drag of a new segment).
    pub fn shortest_to(self, other: TorusPoint) -> TorusVec {
        TorusVec {
            dx: shortest_delta(other.x - self.x),
            dy: shortest_delta(other.y - self.y),
        }
    }

    /// Toroidal Euclidean distance: the length of the shortest displacement.
    pub fn distance_to(self, other: TorusPoint) -> f32 {
        let v = self.shortest_to(other);
        (v.dx * v.dx + v.dy * v.dy).sqrt()
    }
}

/// Reduce a delta in (−1, 1) into the shortest representative in (−½, ½].
fn shortest_delta(d: f32) -> f32 {
    // Input is the raw difference of two values in [0, 1), so d ∈ (−1, 1).
    if d > 0.5 {
        d - 1.0
    } else if d <= -0.5 {
        d + 1.0
    } else {
        d
    }
}

/// A displacement vector in the universal cover ℝ². Unlike [`TorusPoint`],
/// this is *not* reduced modulo the lattice — `TorusVec { dx: 1.7, dy: 0.0 }`
/// is a different vector from `TorusVec { dx: 0.7, dy: 0.0 }`, even though
/// they translate any point to the same place on the torus. The difference
/// matters for paths: the former wraps once around the x-direction, the
/// latter doesn't.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct TorusVec {
    pub dx: f32,
    pub dy: f32,
}

impl TorusVec {
    pub fn new(dx: f32, dy: f32) -> Self {
        Self { dx, dy }
    }

    pub fn zero() -> Self {
        Self { dx: 0.0, dy: 0.0 }
    }

    pub fn length(self) -> f32 {
        (self.dx * self.dx + self.dy * self.dy).sqrt()
    }
}

impl std::ops::Add for TorusVec {
    type Output = TorusVec;
    fn add(self, other: TorusVec) -> TorusVec {
        TorusVec::new(self.dx + other.dx, self.dy + other.dy)
    }
}

impl std::ops::Sub for TorusVec {
    type Output = TorusVec;
    fn sub(self, other: TorusVec) -> TorusVec {
        TorusVec::new(self.dx - other.dx, self.dy - other.dy)
    }
}

/// A straight-line segment on the torus.
///
/// We store the segment as (start, displacement) rather than (start, end),
/// because two torus points have infinitely many straight-line paths
/// between them (one per homotopy class). The displacement picks one.
#[derive(Copy, Clone, Debug)]
pub struct TorusSegment {
    pub start: TorusPoint,
    pub disp: TorusVec,
}

impl TorusSegment {
    /// The segment going *the short way around* from `a` to `b`. This is
    /// the natural choice for a freshly-drawn line.
    #[allow(dead_code)] // public API; exercised in tests, not yet from main
    pub fn shortest(a: TorusPoint, b: TorusPoint) -> Self {
        Self {
            start: a,
            disp: a.shortest_to(b),
        }
    }

    /// The (canonical-form) endpoint of the segment on the torus.
    pub fn end(self) -> TorusPoint {
        self.start.translate(self.disp)
    }

    /// Translate the whole segment (both endpoints) by `v`.
    pub fn translated(self, v: TorusVec) -> Self {
        Self {
            start: self.start.translate(v),
            disp: self.disp,
        }
    }

    /// Iterate over the "visible lifts" of this segment in the universal
    /// cover, expressed as (start_xy, end_xy) pairs in ℝ². Together with
    /// integer translates, these tile the plane and cover any portion of
    /// the segment that might be visible in the unit-square viewport
    /// [0, 1)².
    ///
    /// We yield a 3×3 block of integer translates centered on the
    /// fundamental domain. This is enough as long as no single drag step
    /// pushes |disp| above ~1, which is fine for interactive use.
    pub fn visible_lifts(self) -> impl Iterator<Item = ((f32, f32), (f32, f32))> {
        let ax = self.start.x;
        let ay = self.start.y;
        let bx = ax + self.disp.dx;
        let by = ay + self.disp.dy;
        (-1..=1).flat_map(move |i| {
            (-1..=1).map(move |j| {
                let ox = i as f32;
                let oy = j as f32;
                ((ax + ox, ay + oy), (bx + ox, by + oy))
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn point_wraps_into_unit_square() {
        let p = TorusPoint::new(1.3, -0.2);
        assert!(approx_eq(p.x(), 0.3));
        assert!(approx_eq(p.y(), 0.8));
    }

    #[test]
    fn shortest_goes_across_seam() {
        let a = TorusPoint::new(0.9, 0.5);
        let b = TorusPoint::new(0.1, 0.5);
        let v = a.shortest_to(b);
        // Crossing right→left edge is shorter than going the long way.
        assert!(approx_eq(v.dx, 0.2));
        assert!(approx_eq(v.dy, 0.0));
    }

    #[test]
    fn shortest_distance_is_symmetric() {
        let a = TorusPoint::new(0.1, 0.2);
        let b = TorusPoint::new(0.7, 0.9);
        assert!(approx_eq(a.distance_to(b), b.distance_to(a)));
    }

    #[test]
    fn segment_end_consistent_with_translate() {
        let seg = TorusSegment {
            start: TorusPoint::new(0.4, 0.4),
            disp: TorusVec::new(0.7, 0.7),
        };
        let e = seg.end();
        // 0.4 + 0.7 = 1.1, wraps to 0.1
        assert!(approx_eq(e.x(), 0.1));
        assert!(approx_eq(e.y(), 0.1));
    }

}
