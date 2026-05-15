//! Glyphs: connected drawings on the torus, with a DCEL for their faces.
//!
//! A `Glyph` is a set of `TorusSegment`s that the user has drawn together
//! (either directly or via overlap-induced merging). The `Dcel` is a
//! derived structure: vertices live at segment endpoints AND at every
//! segment-segment crossing on the torus, edges are the resulting
//! split sub-segments, and faces are the cycles produced by the standard
//! half-edge face-tracing convention.
//!
//! Faces are traced in the universal cover ℝ². A face whose boundary
//! walk closes (Σ disp ≈ 0) is rendered as a polygon (tiled across the
//! viewport). A face with positive signed area fills its polygon
//! interior; a face with negative signed area fills the *complement*
//! (the entire canvas), so a small loop on the torus paints its inside
//! with one color and "everywhere else" with another. Non-contractible
//! faces (boundary walk lands on a different lift) are skipped — they're
//! cylinders, not disks, and don't have a sensible polygon fill.

use macroquad::prelude::Color;

use crate::torus::{TorusPoint, TorusSegment, TorusVec};

/// Vertex-merge tolerance in torus coordinates (≈ one pixel at 900px).
const EPS: f32 = 1.0 / 1024.0;

pub struct Glyph {
    pub segments: Vec<TorusSegment>,
    pub dcel: Dcel,
    /// Number of connected components of T² \ glyph. This is the
    /// *topological* face count, and differs from the DCEL's
    /// combinatorial face count whenever the graph has non-separating
    /// cycles (e.g. a single non-trivial loop on the torus splits into
    /// 2 combinatorial DCEL face cycles but leaves 1 topological face).
    pub topological_face_count: usize,
}

impl Glyph {
    pub fn from_segments(segments: Vec<TorusSegment>) -> Self {
        let dcel = Dcel::build(&segments);
        let topological_face_count = compute_topological_face_count(&segments);
        Self {
            segments,
            dcel,
            topological_face_count,
        }
    }
}

const RASTER_GRID: usize = 256;

/// Estimate the number of connected components of T² \ segments by
/// rasterizing the segments into a square bitmap (with the torus's
/// wrap-around treated as 4-connected pixel adjacency) and counting
/// connected blocks of unblocked pixels.
///
/// At GRID = 256, two parallel segments closer than ~1/256 ≈ 0.004 of
/// the viewport may merge after rasterization and undercount their
/// enclosed strip, but for typical user-drawn segments this is fine.
fn compute_topological_face_count(segments: &[TorusSegment]) -> usize {
    if segments.is_empty() {
        return 1;
    }
    let n = RASTER_GRID;
    let mut blocked = vec![false; n * n];
    let g = n as f32;
    for seg in segments {
        let len = seg.disp.dx.abs().max(seg.disp.dy.abs()).max(0.001);
        let steps = ((len * g) as usize).max(2) * 3;
        let sx = seg.start.x();
        let sy = seg.start.y();
        for s in 0..=steps {
            let t = s as f32 / steps as f32;
            let x = (sx + t * seg.disp.dx).rem_euclid(1.0);
            let y = (sy + t * seg.disp.dy).rem_euclid(1.0);
            let cx = ((x * g) as usize).min(n - 1);
            let cy = ((y * g) as usize).min(n - 1);
            blocked[cy * n + cx] = true;
        }
    }
    let mut visited = vec![false; n * n];
    let mut count = 0;
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for sy in 0..n {
        for sx in 0..n {
            let i = sy * n + sx;
            if blocked[i] || visited[i] {
                continue;
            }
            visited[i] = true;
            stack.push((sx, sy));
            while let Some((x, y)) = stack.pop() {
                for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let nx = (x as i32 + dx).rem_euclid(n as i32) as usize;
                    let ny = (y as i32 + dy).rem_euclid(n as i32) as usize;
                    let ni = ny * n + nx;
                    if blocked[ni] || visited[ni] {
                        continue;
                    }
                    visited[ni] = true;
                    stack.push((nx, ny));
                }
            }
            count += 1;
        }
    }
    count
}

#[derive(Default)]
pub struct Dcel {
    pub vertices: Vec<TorusPoint>,
    pub half_edges: Vec<HalfEdge>,
    pub faces: Vec<Face>,
}

#[derive(Clone)]
pub struct HalfEdge {
    pub origin: usize,
    pub twin: usize,
    pub next: usize,
    pub face: usize,
    /// Displacement from origin's lift to the destination's lift, in ℝ².
    /// This is the exact direction the edge runs, not the shortest-on-torus
    /// shortcut — required for correct face tracing across the seam.
    pub disp: TorusVec,
}

pub struct Face {
    /// Polygon traced in the universal cover, starting at some vertex
    /// lift. Empty if the boundary walk didn't close on the same lift
    /// (i.e. the face is non-contractible).
    pub polygon: Vec<(f32, f32)>,
    pub signed_area: f32,
    pub color: Color,
}

impl Dcel {
    pub fn build(segments: &[TorusSegment]) -> Self {
        if segments.is_empty() {
            return Self::default();
        }

        // Each "crossing" on segment i is a (t, lifted_xy) pair. t is in
        // [0, 1] along the segment; lifted_xy is the point's position in
        // the segment's own canonical lift (start at (start.x(), start.y())).
        let mut crossings: Vec<Vec<(f32, (f32, f32))>> = vec![Vec::new(); segments.len()];

        // Always include each segment's two endpoints.
        for (i, seg) in segments.iter().enumerate() {
            let sx = seg.start.x();
            let sy = seg.start.y();
            crossings[i].push((0.0, (sx, sy)));
            crossings[i].push((1.0, (sx + seg.disp.dx, sy + seg.disp.dy)));
        }

        // Find pairwise intersections on the torus by lifting seg_j to
        // every neighboring integer translate and intersecting against
        // seg_i in its canonical lift.
        for i in 0..segments.len() {
            for j in i..segments.len() {
                let seg_a = &segments[i];
                let seg_b = &segments[j];
                let a0 = (seg_a.start.x(), seg_a.start.y());
                let a1 = (a0.0 + seg_a.disp.dx, a0.1 + seg_a.disp.dy);
                let b0c = (seg_b.start.x(), seg_b.start.y());
                let b1c = (b0c.0 + seg_b.disp.dx, b0c.1 + seg_b.disp.dy);
                for li in -1..=1 {
                    for lj in -1..=1 {
                        if i == j && li == 0 && lj == 0 {
                            continue;
                        }
                        let ox = li as f32;
                        let oy = lj as f32;
                        let b0 = (b0c.0 + ox, b0c.1 + oy);
                        let b1 = (b1c.0 + ox, b1c.1 + oy);
                        let Some((ta, tb, px, py)) = seg_seg_intersect(a0, a1, b0, b1) else {
                            continue;
                        };
                        if ta > EPS && ta < 1.0 - EPS {
                            crossings[i].push((ta, (px, py)));
                        }
                        // For seg_j, the intersection point in its own
                        // canonical lift is (px, py) minus the offset (ox, oy).
                        if tb > EPS && tb < 1.0 - EPS {
                            crossings[j].push((tb, (px - ox, py - oy)));
                        }
                    }
                }
            }
        }

        // Sort crossings along each segment.
        for cs in crossings.iter_mut() {
            cs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        }

        // Build the vertex list with epsilon-merging by canonical position.
        let mut vertices: Vec<TorusPoint> = Vec::new();
        // For each segment, the sequence of vertex indices at its crossings.
        let mut seg_vertex_seqs: Vec<Vec<usize>> = Vec::with_capacity(segments.len());
        // Also store the lifted position used for each crossing, so we
        // can compute exact sub-segment displacements later.
        let mut seg_lifted_seqs: Vec<Vec<(f32, f32)>> = Vec::with_capacity(segments.len());

        for cs in &crossings {
            let mut vseq = Vec::with_capacity(cs.len());
            let mut lseq = Vec::with_capacity(cs.len());
            let mut last_lift: Option<(f32, f32)> = None;
            for &(_, (lx, ly)) in cs {
                // Dedup only on lifted position (NOT canonical vertex),
                // so a segment that closes on itself across a wrap —
                // e.g. start (0, 0.5), disp (1, 0) — still produces two
                // distinct sequence entries at the same vertex, giving
                // a self-loop edge.
                if let Some((px, py)) = last_lift {
                    if (px - lx).abs() < EPS && (py - ly).abs() < EPS {
                        continue;
                    }
                }
                let p = TorusPoint::new(lx, ly);
                let v = find_or_insert_vertex(&mut vertices, p);
                vseq.push(v);
                lseq.push((lx, ly));
                last_lift = Some((lx, ly));
            }
            seg_vertex_seqs.push(vseq);
            seg_lifted_seqs.push(lseq);
        }

        // Build half-edges. Each sub-segment u→v contributes a half-edge
        // pair with displacement equal to the exact lifted offset.
        let mut half_edges: Vec<HalfEdge> = Vec::new();
        for s in 0..segments.len() {
            let vs = &seg_vertex_seqs[s];
            let ls = &seg_lifted_seqs[s];
            for k in 0..vs.len().saturating_sub(1) {
                let u = vs[k];
                let v = vs[k + 1];
                let dx = ls[k + 1].0 - ls[k].0;
                let dy = ls[k + 1].1 - ls[k].1;
                // Only drop sub-segments with zero lifted length. Self-
                // loops (u == v but non-zero displacement, i.e. an edge
                // wrapping around the torus back to its own vertex) are
                // legal and must be kept.
                if dx * dx + dy * dy < EPS * EPS {
                    continue;
                }
                let h0 = half_edges.len();
                let h1 = h0 + 1;
                half_edges.push(HalfEdge {
                    origin: u,
                    twin: h1,
                    next: usize::MAX,
                    face: usize::MAX,
                    disp: TorusVec::new(dx, dy),
                });
                half_edges.push(HalfEdge {
                    origin: v,
                    twin: h0,
                    next: usize::MAX,
                    face: usize::MAX,
                    disp: TorusVec::new(-dx, -dy),
                });
            }
        }

        // Group outgoing half-edges per vertex, sorted by angle.
        let mut outgoing: Vec<Vec<usize>> = vec![Vec::new(); vertices.len()];
        for (hi, h) in half_edges.iter().enumerate() {
            outgoing[h.origin].push(hi);
        }
        for he_list in outgoing.iter_mut() {
            he_list.sort_by(|&a, &b| {
                let ha = &half_edges[a];
                let hb = &half_edges[b];
                let aa = ha.disp.dy.atan2(ha.disp.dx);
                let ab = hb.disp.dy.atan2(hb.disp.dx);
                aa.partial_cmp(&ab).unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        // For each half-edge, remember its index within its origin's outgoing list.
        let mut out_idx: Vec<usize> = vec![0; half_edges.len()];
        for (v, list) in outgoing.iter().enumerate() {
            for (k, &h) in list.iter().enumerate() {
                let _ = v;
                out_idx[h] = k;
            }
        }

        // Standard DCEL face-next: next(h) is the half-edge immediately
        // *clockwise* of twin(h) at h's destination vertex.
        for hi in 0..half_edges.len() {
            let twin = half_edges[hi].twin;
            let v = half_edges[twin].origin; // = destination(hi)
            let list = &outgoing[v];
            let k = out_idx[twin];
            let n = list.len();
            let prev = list[(k + n - 1) % n];
            half_edges[hi].next = prev;
        }

        // Trace faces.
        let mut faces: Vec<Face> = Vec::new();
        let mut visited = vec![false; half_edges.len()];
        for start in 0..half_edges.len() {
            if visited[start] {
                continue;
            }
            let face_idx = faces.len();
            let mut cycle: Vec<usize> = Vec::new();
            let mut h = start;
            let mut guard = 0;
            loop {
                if visited[h] || guard > half_edges.len() * 4 {
                    break;
                }
                visited[h] = true;
                cycle.push(h);
                half_edges[h].face = face_idx;
                h = half_edges[h].next;
                if h == start {
                    break;
                }
                guard += 1;
            }

            // Trace polygon in ℝ², checking whether it closes.
            let start_v = half_edges[cycle[0]].origin;
            let (vx0, vy0) = (vertices[start_v].x(), vertices[start_v].y());
            let mut polygon: Vec<(f32, f32)> = Vec::with_capacity(cycle.len());
            let mut x = vx0;
            let mut y = vy0;
            polygon.push((x, y));
            let mut total = TorusVec::zero();
            for &he in &cycle {
                let d = half_edges[he].disp;
                x += d.dx;
                y += d.dy;
                total = total + d;
                polygon.push((x, y));
            }
            let closes = total.dx.abs() < EPS && total.dy.abs() < EPS;
            if closes {
                // Drop the duplicated closing vertex.
                polygon.pop();
            } else {
                polygon.clear();
            }
            let signed_area = polygon_signed_area(&polygon);
            let color = face_color(&polygon);
            faces.push(Face {
                polygon,
                signed_area,
                color,
            });
        }

        Self {
            vertices,
            half_edges,
            faces,
        }
    }
}

fn find_or_insert_vertex(vs: &mut Vec<TorusPoint>, p: TorusPoint) -> usize {
    for (i, q) in vs.iter().enumerate() {
        if p.distance_to(*q) <= EPS {
            return i;
        }
    }
    vs.push(p);
    vs.len() - 1
}

/// 2D segment-segment intersection in ℝ². Returns (t_along_a, t_along_b,
/// intersection_x, intersection_y) if the segments cross (parameters in
/// [0, 1]). Parallel/collinear segments are reported as no intersection.
fn seg_seg_intersect(
    a0: (f32, f32),
    a1: (f32, f32),
    b0: (f32, f32),
    b1: (f32, f32),
) -> Option<(f32, f32, f32, f32)> {
    let dax = a1.0 - a0.0;
    let day = a1.1 - a0.1;
    let dbx = b1.0 - b0.0;
    let dby = b1.1 - b0.1;
    let denom = dax * dby - day * dbx;
    if denom.abs() < 1e-12 {
        return None;
    }
    let ex = b0.0 - a0.0;
    let ey = b0.1 - a0.1;
    let ta = (ex * dby - ey * dbx) / denom;
    let tb = (ex * day - ey * dax) / denom;
    if !(-EPS..=1.0 + EPS).contains(&ta) || !(-EPS..=1.0 + EPS).contains(&tb) {
        return None;
    }
    let px = a0.0 + ta * dax;
    let py = a0.1 + ta * day;
    Some((ta, tb, px, py))
}

fn polygon_signed_area(poly: &[(f32, f32)]) -> f32 {
    if poly.len() < 3 {
        return 0.0;
    }
    let mut s = 0.0;
    let n = poly.len();
    for i in 0..n {
        let (x0, y0) = poly[i];
        let (x1, y1) = poly[(i + 1) % n];
        s += x0 * y1 - x1 * y0;
    }
    s * 0.5
}

/// Stable pseudo-random color per face, derived from its boundary
/// polygon (so face colors persist across re-renders that don't
/// actually change the face).
fn face_color(poly: &[(f32, f32)]) -> Color {
    if poly.is_empty() {
        // Won't be rendered, but pick something.
        return Color::new(0.0, 0.0, 0.0, 0.0);
    }
    // Quantize positions and find the cyclic rotation starting at the
    // minimum; this makes the fingerprint independent of which half-edge
    // we started the face trace from.
    let q: Vec<(i32, i32)> = poly
        .iter()
        .map(|&(x, y)| ((x * 4096.0).round() as i32, (y * 4096.0).round() as i32))
        .collect();
    let n = q.len();
    let mut min_i = 0;
    for i in 1..n {
        if q[i] < q[min_i] {
            min_i = i;
        }
    }
    let mut h: u64 = 0xcbf29ce484222325;
    for k in 0..n {
        let (x, y) = q[(min_i + k) % n];
        h = h.wrapping_mul(0x100000001b3) ^ (x as u32 as u64);
        h = h.wrapping_mul(0x100000001b3) ^ (y as u32 as u64);
    }
    let hue = (h % 360) as f32;
    hsv_to_rgb(hue, 0.55, 0.95, 0.18)
}

fn hsv_to_rgb(h_deg: f32, s: f32, v: f32, a: f32) -> Color {
    let c = v * s;
    let h = (h_deg / 60.0).rem_euclid(6.0);
    let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    Color::new(r + m, g + m, b + m, a)
}

/// Does this segment, lifted to the torus, geometrically touch any
/// segment of `glyph`? Used to decide whether a freshly-drawn segment
/// should be merged into an existing glyph. We check all 9 lifts of
/// `seg_b` against `seg_a`'s canonical lift, so endpoints that wrap
/// around the seam still register.
pub fn segment_touches_glyph(new_seg: &TorusSegment, glyph: &Glyph) -> bool {
    for seg in &glyph.segments {
        if segments_touch(new_seg, seg) {
            return true;
        }
    }
    false
}

fn segments_touch(seg_a: &TorusSegment, seg_b: &TorusSegment) -> bool {
    let a0 = (seg_a.start.x(), seg_a.start.y());
    let a1 = (a0.0 + seg_a.disp.dx, a0.1 + seg_a.disp.dy);
    let b0c = (seg_b.start.x(), seg_b.start.y());
    let b1c = (b0c.0 + seg_b.disp.dx, b0c.1 + seg_b.disp.dy);
    for li in -1..=1 {
        for lj in -1..=1 {
            let ox = li as f32;
            let oy = lj as f32;
            let b0 = (b0c.0 + ox, b0c.1 + oy);
            let b1 = (b1c.0 + ox, b1c.1 + oy);
            if seg_seg_intersect(a0, a1, b0, b1).is_some() {
                return true;
            }
        }
    }
    false
}

/// Triangulate a polygon by ear-clipping. Handles "weakly simple"
/// polygons (where the same vertex position appears more than once,
/// e.g. a polygon with a dangling-edge spike) by splitting at the
/// duplicate pair into two simpler polygons and recursing.
pub fn triangulate(poly: &[(f32, f32)]) -> Vec<[(f32, f32); 3]> {
    let n = poly.len();
    if n < 3 {
        return Vec::new();
    }
    // Find any pair of equal positions at non-adjacent indices and
    // split: part A goes around the spike, part B is the spike itself
    // (degenerate, area ≈ 0, contributes nothing). This is what makes
    // face polygons with dangling edges renderable.
    for i in 0..n {
        for j in (i + 2)..n {
            if i == 0 && j == n - 1 {
                // Cyclically adjacent — first and last vertex of the
                // polygon are the same position. Not a true spike.
                continue;
            }
            if (poly[i].0 - poly[j].0).abs() < EPS
                && (poly[i].1 - poly[j].1).abs() < EPS
            {
                let mut part_a: Vec<(f32, f32)> = poly[..=i].to_vec();
                part_a.extend_from_slice(&poly[(j + 1)..]);
                let part_b: Vec<(f32, f32)> = poly[i..=j].to_vec();
                let mut out = triangulate(&part_a);
                out.extend(triangulate(&part_b));
                return out;
            }
        }
    }
    triangulate_simple(poly)
}

fn triangulate_simple(poly: &[(f32, f32)]) -> Vec<[(f32, f32); 3]> {
    let n = poly.len();
    if n < 3 {
        return Vec::new();
    }
    let mut pts: Vec<(f32, f32)> = poly.to_vec();
    // Force CCW orientation for the standard ear test.
    if polygon_signed_area(&pts) < 0.0 {
        pts.reverse();
    }
    let mut idx: Vec<usize> = (0..pts.len()).collect();
    let mut out: Vec<[(f32, f32); 3]> = Vec::with_capacity(n - 2);
    let mut guard = 0;
    while idx.len() > 3 {
        guard += 1;
        if guard > n * n + 10 {
            return out; // bail out on degenerate input
        }
        let len = idx.len();
        let mut ear: Option<usize> = None;
        for i in 0..len {
            let ip = idx[(i + len - 1) % len];
            let ic = idx[i];
            let in_ = idx[(i + 1) % len];
            let a = pts[ip];
            let b = pts[ic];
            let c = pts[in_];
            let cross = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
            if cross <= 0.0 {
                continue; // reflex or collinear
            }
            let mut contains = false;
            for j in 0..len {
                let jj = idx[j];
                if jj == ip || jj == ic || jj == in_ {
                    continue;
                }
                if point_in_triangle(pts[jj], a, b, c) {
                    contains = true;
                    break;
                }
            }
            if !contains {
                ear = Some(i);
                break;
            }
        }
        let Some(i) = ear else {
            return out;
        };
        let ip = idx[(i + idx.len() - 1) % idx.len()];
        let ic = idx[i];
        let in_ = idx[(i + 1) % idx.len()];
        out.push([pts[ip], pts[ic], pts[in_]]);
        idx.remove(i);
    }
    if idx.len() == 3 {
        out.push([pts[idx[0]], pts[idx[1]], pts[idx[2]]]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(ax: f32, ay: f32, bx: f32, by: f32) -> TorusSegment {
        let start = TorusPoint::new(ax, ay);
        TorusSegment {
            start,
            disp: TorusVec::new(bx - ax, by - ay),
        }
    }

    #[test]
    fn empty_dcel() {
        let d = Dcel::build(&[]);
        assert!(d.vertices.is_empty());
        assert!(d.faces.is_empty());
    }

    #[test]
    fn single_segment_has_one_non_contractible_face() {
        // A single edge in any cellular sense gives just one combinatorial
        // face whose boundary traverses the edge twice in opposite
        // directions — total disp = 0 — so the polygon "closes" but is
        // degenerate (collinear). It either has empty polygon (signed
        // area 0) or near-zero area; either way nothing visible.
        let d = Dcel::build(&[seg(0.3, 0.3, 0.7, 0.5)]);
        assert_eq!(d.vertices.len(), 2);
        assert_eq!(d.half_edges.len(), 2);
        assert_eq!(d.faces.len(), 1);
        assert!(d.faces[0].signed_area.abs() < 1e-4);
    }

    #[test]
    fn triangle_yields_inner_and_outer_faces() {
        let segs = vec![
            seg(0.4, 0.4, 0.6, 0.4),
            seg(0.6, 0.4, 0.5, 0.6),
            seg(0.5, 0.6, 0.4, 0.4),
        ];
        let d = Dcel::build(&segs);
        assert_eq!(d.vertices.len(), 3);
        assert_eq!(d.half_edges.len(), 6);
        assert_eq!(d.faces.len(), 2);
        let signs: Vec<f32> = d.faces.iter().map(|f| f.signed_area).collect();
        // One positive (inner), one negative (outer), equal magnitude.
        assert!(signs.iter().any(|&s| s > 1e-4));
        assert!(signs.iter().any(|&s| s < -1e-4));
        let sum: f32 = signs.iter().sum();
        assert!(sum.abs() < 1e-4);
    }

    #[test]
    fn crossing_segments_create_intersection_vertex() {
        let segs = vec![
            seg(0.3, 0.5, 0.7, 0.5), // horizontal
            seg(0.5, 0.3, 0.5, 0.7), // vertical, crosses at (0.5, 0.5)
        ];
        let d = Dcel::build(&segs);
        // 4 endpoints + 1 intersection = 5 vertices.
        assert_eq!(d.vertices.len(), 5);
        // 4 sub-segments after splitting × 2 half-edges = 8.
        assert_eq!(d.half_edges.len(), 8);
    }

    #[test]
    fn touching_segments_detected() {
        let a = seg(0.3, 0.5, 0.7, 0.5);
        let b = seg(0.5, 0.3, 0.5, 0.7);
        let g = Glyph::from_segments(vec![a]);
        assert!(segment_touches_glyph(&b, &g));
        // A far-away segment doesn't touch.
        let c = seg(0.05, 0.05, 0.1, 0.1);
        assert!(!segment_touches_glyph(&c, &g));
    }

    #[test]
    fn longitude_loop_has_two_non_contractible_faces() {
        // A single edge wrapping around the longitude: start at (0.0, 0.5),
        // disp (1.0, 0.0) — its endpoint coincides with its start on the
        // torus, so the DCEL collapses to one vertex with one self-loop edge.
        let s = seg(0.0, 0.5, 1.0, 0.5);
        let d = Dcel::build(&[s]);
        assert_eq!(d.vertices.len(), 1);
        assert_eq!(d.half_edges.len(), 2);
        assert_eq!(d.faces.len(), 2);
        // Both faces should be non-contractible (no polygon).
        assert!(d.faces.iter().all(|f| f.polygon.is_empty()));
    }

    #[test]
    fn topological_meridian_does_not_separate_torus() {
        // A meridian loop is non-separating: cutting T² along it
        // gives a single open cylinder, not two halves. The DCEL has
        // 2 combinatorial face cycles, but the topology has 1 face.
        let s = seg(0.0, 0.5, 1.0, 0.5);
        let g = Glyph::from_segments(vec![s]);
        assert_eq!(g.dcel.faces.len(), 2, "combinatorial face count");
        assert_eq!(
            g.topological_face_count, 1,
            "meridian loop is non-separating"
        );
    }

    #[test]
    fn topological_diagonal_loop_has_one_face() {
        // A (1,1) torus knot is non-separating: it has 2 combinatorial
        // DCEL faces but only 1 topological face.
        let s = seg(0.0, 0.0, 1.0, 1.0);
        let g = Glyph::from_segments(vec![s]);
        assert_eq!(g.dcel.faces.len(), 2, "expected 2 combinatorial faces");
        assert_eq!(
            g.topological_face_count, 1,
            "expected 1 topological face (loop doesn't separate the torus)"
        );
    }

    #[test]
    fn topological_triangle_has_two_faces() {
        let segs = vec![
            seg(0.4, 0.4, 0.6, 0.4),
            seg(0.6, 0.4, 0.5, 0.6),
            seg(0.5, 0.6, 0.4, 0.4),
        ];
        let g = Glyph::from_segments(segs);
        assert_eq!(g.topological_face_count, 2);
    }

    #[test]
    fn triangle_plus_crossing_segment_has_two_faces() {
        // Triangle ABC plus a segment EF that crosses edge AB at D.
        // E is above (outside) the triangle, F is below the top edge (inside).
        // The new segment ends up "partly in, partly out" of the triangle.
        let segs = vec![
            seg(0.3, 0.3, 0.7, 0.3), // A→B (top)
            seg(0.7, 0.3, 0.5, 0.7), // B→C
            seg(0.5, 0.7, 0.3, 0.3), // C→A
            seg(0.5, 0.1, 0.5, 0.5), // E→F, crosses AB at (0.5, 0.3)
        ];
        let d = Dcel::build(&segs);
        // 6 vertices: A, B, C, D (intersection), E, F.
        assert_eq!(d.vertices.len(), 6, "vertex count");
        // 2 faces: an inner one containing the triangle, an outer one.
        // (The two "dangling" half-edges off D — one into the triangle,
        // one outside — don't split either face; they just elongate the
        // face boundary cycle.)
        assert_eq!(d.faces.len(), 2, "face count");
        let positive = d.faces.iter().filter(|f| f.signed_area > 1e-4).count();
        let negative = d.faces.iter().filter(|f| f.signed_area < -1e-4).count();
        assert_eq!(positive, 1, "expected one positive-area face");
        assert_eq!(negative, 1, "expected one negative-area face");
        // Sanity: face colors are different so they're visually distinguishable.
        let ca = d.faces[0].color;
        let cb = d.faces[1].color;
        let same = (ca.r - cb.r).abs() < 1e-3
            && (ca.g - cb.g).abs() < 1e-3
            && (ca.b - cb.b).abs() < 1e-3;
        assert!(!same, "expected distinct face colors");

        // The inner face polygon includes a "spike" (the dangling edge
        // traversed once in each direction), so a vertex appears twice
        // in the boundary. Make sure the renderer can still triangulate
        // it — otherwise the face draws nothing.
        let inner = d
            .faces
            .iter()
            .find(|f| f.signed_area > 0.0)
            .expect("inner face");
        let tris = triangulate(&inner.polygon);
        assert!(
            !tris.is_empty(),
            "ear-clip produced no triangles for a non-degenerate inner face"
        );
    }
}

fn point_in_triangle(p: (f32, f32), a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let sign = |p: (f32, f32), q: (f32, f32), r: (f32, f32)| {
        (p.0 - r.0) * (q.1 - r.1) - (q.0 - r.0) * (p.1 - r.1)
    };
    let d1 = sign(p, a, b);
    let d2 = sign(p, b, c);
    let d3 = sign(p, c, a);
    let has_neg = (d1 < 0.0) || (d2 < 0.0) || (d3 < 0.0);
    let has_pos = (d1 > 0.0) || (d2 > 0.0) || (d3 > 0.0);
    !(has_neg && has_pos)
}
