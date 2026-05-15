//! Glyphs: connected drawings on the torus, with a DCEL for their faces.
//!
//! # Identity model
//!
//! A glyph is a graph: a list of vertices with stable IDs (`VertexId`)
//! and a list of edges referencing those vertices by ID. Each edge
//! also carries a *winding* — an integer (wx, wy) — so the destination
//! vertex's lift in ℝ² is `v.pos + winding`. The geometric
//! displacement of an edge is fully determined by the two vertex
//! positions and the winding; we never store float displacements on
//! the edge itself.
//!
//! This is the key invariant that fixes the f32-drift class of bugs we
//! were chasing: after a vertex exists, "same vertex" is `==` on
//! `VertexId`, not a re-snap by floating-point position. Floats only
//! enter at the boundary, when an input segment's endpoint is matched
//! to an existing vertex (or a new one is created) by EPS-merging.
//!
//! # Chop invariant
//!
//! After `chop`, no two edges of a glyph cross except at a shared
//! `VertexId` endpoint. The DCEL then derives half-edges and faces from
//! the chopped graph.
//!
//! # Faces
//!
//! Faces are traced in the universal cover ℝ². A face whose boundary
//! walk closes (Σ disp ≈ 0) renders as a polygon; positive signed area
//! fills the polygon interior, negative fills the complement (so a
//! small loop on the torus shows one color inside and one outside).
//! Non-contractible faces (boundary lands on a translated lift) are
//! skipped — they're cylinders, not disks.

use macroquad::prelude::Color;

use crate::torus::{TorusPoint, TorusSegment, TorusVec};

/// Vertex-merge tolerance in torus coordinates (≈ one pixel at 900px).
const EPS: f32 = 1.0 / 1024.0;

pub type VertexId = u32;

pub struct Glyph {
    pub vertices: Vec<TorusPoint>,
    pub edges: Vec<Edge>,
    pub half_edges: Vec<HalfEdge>,
    pub faces: Vec<Face>,
    pub topological_face_count: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct Edge {
    pub u: VertexId,
    pub v: VertexId,
    /// Integer lift offset for the edge from `u` to `v`: in the
    /// universal cover, `u` is at its canonical position and `v` is at
    /// `v.pos + winding`. (0, 0) for an edge that doesn't wrap.
    pub winding: (i32, i32),
}

#[derive(Clone)]
pub struct HalfEdge {
    pub origin: VertexId,
    pub edge: u32,
    pub twin: u32,
    pub next: u32,
    pub face: u32,
}

pub struct Face {
    /// Polygon traced in the universal cover. Empty if the boundary
    /// walk didn't close on the same lift (non-contractible face).
    pub polygon: Vec<(f32, f32)>,
    pub signed_area: f32,
    pub color: Color,
}

impl Glyph {
    /// Build a glyph from a flat segment list. Endpoints are EPS-merged
    /// into vertices; intersections are computed and the graph is
    /// chopped so no two edges share an interior point.
    pub fn from_segments(segments: Vec<TorusSegment>) -> Self {
        let mut vertices: Vec<TorusPoint> = Vec::new();
        let mut edges: Vec<Edge> = Vec::with_capacity(segments.len());
        for s in &segments {
            let u_id = find_or_insert_vertex(&mut vertices, s.start);
            let v_id = find_or_insert_vertex(&mut vertices, s.end());
            let winding = winding_from_disp(s.start, s.end(), s.disp);
            edges.push(Edge {
                u: u_id,
                v: v_id,
                winding,
            });
        }
        chop(&mut vertices, &mut edges);
        Self::assemble(vertices, edges)
    }

    /// Build a glyph from already-prepared vertices and edges. The
    /// inputs are NOT re-chopped; the caller is responsible for the
    /// no-interior-crossings invariant. Used by `partition_into_glyphs`
    /// to construct each connected component without redundant work.
    fn assemble(vertices: Vec<TorusPoint>, edges: Vec<Edge>) -> Self {
        let (half_edges, faces) = build_half_edges_and_faces(&vertices, &edges);
        let topological_face_count = compute_topological_face_count(&vertices, &edges);
        Self {
            vertices,
            edges,
            half_edges,
            faces,
            topological_face_count,
        }
    }

    /// Reconstruct an edge's geometric `TorusSegment` for drawing /
    /// hit-testing. Defined as: starts at u's canonical position,
    /// disp = u.signed_diff_to(v) + winding.
    pub fn edge_segment(&self, edge_idx: usize) -> TorusSegment {
        let e = &self.edges[edge_idx];
        let u = self.vertices[e.u as usize];
        let v = self.vertices[e.v as usize];
        TorusSegment {
            start: u,
            disp: TorusVec::new(
                u.x.signed_diff_to(v.x) + e.winding.0 as f32,
                u.y.signed_diff_to(v.y) + e.winding.1 as f32,
            ),
        }
    }

    /// All edges as TorusSegments. Convenience for snapshotting,
    /// drawing, and partitioning.
    pub fn segments(&self) -> Vec<TorusSegment> {
        (0..self.edges.len()).map(|i| self.edge_segment(i)).collect()
    }

    /// Translate every vertex of this glyph by `delta`. Edges and
    /// windings stay the same (relative geometry is invariant under
    /// translation); half-edges and faces are rebuilt so face overlays
    /// follow the move. No re-chop is needed because translation
    /// doesn't introduce or remove intersections among the glyph's own
    /// edges, and (by design) we don't change glyph membership on
    /// drag.
    pub fn translate(&mut self, delta: TorusVec) {
        for v in &mut self.vertices {
            *v = v.translate(delta);
        }
        let (half_edges, faces) = build_half_edges_and_faces(&self.vertices, &self.edges);
        self.half_edges = half_edges;
        self.faces = faces;
    }
}

/// Add a freshly-drawn segment to the world: merge it into every glyph
/// it touches (or start a new singleton glyph). The result is chopped
/// so the no-interior-crossings invariant holds.
pub fn add_segment(glyphs: &mut Vec<Glyph>, seg: TorusSegment) {
    let mut touched: Vec<usize> = (0..glyphs.len())
        .filter(|&i| segment_touches_glyph(&seg, &glyphs[i]))
        .collect();
    let mut combined: Vec<TorusSegment> = vec![seg];
    touched.sort();
    for &i in touched.iter().rev() {
        combined.extend(glyphs.remove(i).segments());
    }
    glyphs.push(Glyph::from_segments(combined));
}

/// Remove one edge of a glyph. If that disconnects the graph, the
/// remaining edges are re-partitioned into separate connected glyphs.
pub fn remove_edge(glyphs: &mut Vec<Glyph>, glyph_idx: usize, edge_idx: usize) {
    let mut g = glyphs.remove(glyph_idx);
    g.edges.remove(edge_idx);
    if g.edges.is_empty() {
        return;
    }
    // Partition by graph connectivity over (vertex, edge) IDs.
    for (verts, edges) in split_connected_components(g.vertices, g.edges) {
        glyphs.push(Glyph::assemble(verts, edges));
    }
}

/// Split a flat segment list into connected components ("touches"
/// relation: lifted geometric intersection). Used to recover the glyph
/// structure when restoring an undo/redo snapshot.
pub fn partition_into_glyphs(segments: Vec<TorusSegment>) -> Vec<Vec<TorusSegment>> {
    let mut groups: Vec<Vec<TorusSegment>> = Vec::new();
    for s in segments {
        let touched: Vec<usize> = (0..groups.len())
            .filter(|&i| groups[i].iter().any(|t| segments_touch(&s, t)))
            .collect();
        let mut combined = vec![s];
        for &i in touched.iter().rev() {
            combined.extend(groups.remove(i));
        }
        groups.push(combined);
    }
    groups
}

/// Does this segment, lifted to the torus, geometrically touch any
/// segment of `glyph`?
pub fn segment_touches_glyph(new_seg: &TorusSegment, glyph: &Glyph) -> bool {
    for i in 0..glyph.edges.len() {
        if segments_touch(new_seg, &glyph.edge_segment(i)) {
            return true;
        }
    }
    false
}

fn segments_touch(seg_a: &TorusSegment, seg_b: &TorusSegment) -> bool {
    let a0 = (seg_a.start.x.to_f32(), seg_a.start.y.to_f32());
    let a1 = (a0.0 + seg_a.disp.dx, a0.1 + seg_a.disp.dy);
    let b0c = (seg_b.start.x.to_f32(), seg_b.start.y.to_f32());
    let b1c = (b0c.0 + seg_b.disp.dx, b0c.1 + seg_b.disp.dy);
    for li in -1..=1 {
        for lj in -1..=1 {
            let b0 = (b0c.0 + li as f32, b0c.1 + lj as f32);
            let b1 = (b1c.0 + li as f32, b1c.1 + lj as f32);
            if seg_seg_intersect(a0, a1, b0, b1).is_some() {
                return true;
            }
        }
    }
    false
}

// ---------- chop ----------

/// Mutate (vertices, edges) so the no-interior-crossings invariant
/// holds: any pair of edges that intersect in their interiors gets
/// split into pieces meeting at a shared vertex. New vertices are
/// appended to `vertices`; the edge list is replaced with sub-edges.
fn chop(vertices: &mut Vec<TorusPoint>, edges: &mut Vec<Edge>) {
    if edges.is_empty() {
        return;
    }

    // splits[i]: (parameter t, vertex_id) entries along edge i. Always
    // starts with the two endpoints; intersection points get appended.
    let mut splits: Vec<Vec<(f32, VertexId)>> = edges
        .iter()
        .map(|e| vec![(0.0, e.u), (1.0, e.v)])
        .collect();

    // Pairwise intersections across the 3×3 lift block.
    for i in 0..edges.len() {
        for j in i..edges.len() {
            let (i_u_pos, i_dx, i_dy) = edge_lifted_geometry(vertices, edges[i]);
            let (j_u_pos, j_dx, j_dy) = edge_lifted_geometry(vertices, edges[j]);
            let a0 = (i_u_pos.0, i_u_pos.1);
            let a1 = (a0.0 + i_dx, a0.1 + i_dy);
            let b0c = (j_u_pos.0, j_u_pos.1);
            let b1c = (b0c.0 + j_dx, b0c.1 + j_dy);
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
                        let vid = find_or_insert_vertex(vertices, TorusPoint::new(px, py));
                        splits[i].push((ta, vid));
                    }
                    if tb > EPS && tb < 1.0 - EPS {
                        let vid =
                            find_or_insert_vertex(vertices, TorusPoint::new(px - ox, py - oy));
                        splits[j].push((tb, vid));
                    }
                }
            }
        }
    }

    // Defensive vertex-on-segment pass: any vertex that happens to fall
    // on an edge's interior (e.g. from a chained chop where seg-seg
    // intersection landed slightly off-endpoint) should still split the
    // edge.
    for i in 0..edges.len() {
        let (u_pos, di_x, di_y) = edge_lifted_geometry(vertices, edges[i]);
        let len2 = di_x * di_x + di_y * di_y;
        if len2 < EPS * EPS {
            continue;
        }
        for vid in 0..vertices.len() as VertexId {
            if vid == edges[i].u || vid == edges[i].v {
                continue;
            }
            let v = vertices[vid as usize];
            for li in -1..=1 {
                for lj in -1..=1 {
                    let vx = v.x.to_f32() + li as f32;
                    let vy = v.y.to_f32() + lj as f32;
                    let t = ((vx - u_pos.0) * di_x + (vy - u_pos.1) * di_y) / len2;
                    if t <= EPS || t >= 1.0 - EPS {
                        continue;
                    }
                    let proj_x = u_pos.0 + t * di_x;
                    let proj_y = u_pos.1 + t * di_y;
                    let ddx = vx - proj_x;
                    let ddy = vy - proj_y;
                    if ddx * ddx + ddy * ddy < EPS * EPS {
                        splits[i].push((t, vid));
                    }
                }
            }
        }
    }

    // Sort each split list by parameter. Don't dedup by vid: a
    // self-loop legitimately has the same vid at t=0 and t=1, and an
    // edge that wraps and re-enters its start vertex at some interior
    // t can have the same vid more than twice. Degenerate sub-edges
    // (zero lifted length) get filtered when building new_edges below.
    for ss in splits.iter_mut() {
        ss.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    }

    // Rebuild edges from the sub-pieces between consecutive split points.
    let mut new_edges: Vec<Edge> = Vec::with_capacity(edges.len());
    for (i, ss) in splits.iter().enumerate() {
        let (u_pos, di_x, di_y) = edge_lifted_geometry(vertices, edges[i]);
        for k in 0..ss.len().saturating_sub(1) {
            let (t_lo, vid_lo) = ss[k];
            let (t_hi, vid_hi) = ss[k + 1];
            let lo_pos = vertices[vid_lo as usize];
            let hi_pos = vertices[vid_hi as usize];
            let lifted_lo_x = u_pos.0 + t_lo * di_x;
            let lifted_lo_y = u_pos.1 + t_lo * di_y;
            let lifted_hi_x = u_pos.0 + t_hi * di_x;
            let lifted_hi_y = u_pos.1 + t_hi * di_y;
            // Skip degenerate sub-edges (zero lifted length).
            let sub_dx = lifted_hi_x - lifted_lo_x;
            let sub_dy = lifted_hi_y - lifted_lo_y;
            if sub_dx * sub_dx + sub_dy * sub_dy < EPS * EPS {
                continue;
            }
            // Sub-edge in the lifted_disp = signed_diff_to + winding
            // convention: winding = lifted_disp - signed_diff_to.
            let raw_wx = sub_dx - lo_pos.x.signed_diff_to(hi_pos.x);
            let raw_wy = sub_dy - lo_pos.y.signed_diff_to(hi_pos.y);
            new_edges.push(Edge {
                u: vid_lo,
                v: vid_hi,
                winding: (raw_wx.round() as i32, raw_wy.round() as i32),
            });
        }
    }
    *edges = new_edges;
}

/// Returns (u-vertex canonical position, edge dx in ℝ², edge dy in ℝ²)
/// where the edge is laid out with u at its canonical position.
fn edge_lifted_geometry(vertices: &[TorusPoint], e: Edge) -> ((f32, f32), f32, f32) {
    let u = vertices[e.u as usize];
    let v = vertices[e.v as usize];
    let dx = u.x.signed_diff_to(v.x) + e.winding.0 as f32;
    let dy = u.y.signed_diff_to(v.y) + e.winding.1 as f32;
    ((u.x.to_f32(), u.y.to_f32()), dx, dy)
}

fn find_or_insert_vertex(vs: &mut Vec<TorusPoint>, p: TorusPoint) -> VertexId {
    for (i, q) in vs.iter().enumerate() {
        if p.distance_to(*q) <= EPS {
            return i as VertexId;
        }
    }
    vs.push(p);
    (vs.len() - 1) as VertexId
}

/// Given a segment drawn with start S, canonical end E, and lifted
/// disp, return the integer winding. Convention: the lifted disp is
/// `start.signed_diff_to(end) + winding`, where `signed_diff_to` is
/// translation-invariant, so winding is too. (Round to suppress f32
/// noise on the integer.)
fn winding_from_disp(start: TorusPoint, end: TorusPoint, disp: TorusVec) -> (i32, i32) {
    let sd_x = start.x.signed_diff_to(end.x);
    let sd_y = start.y.signed_diff_to(end.y);
    ((disp.dx - sd_x).round() as i32, (disp.dy - sd_y).round() as i32)
}

/// 2D segment-segment intersection in ℝ², parameterized along both
/// segments. Returns (ta, tb, px, py) if the intersection lies within
/// [-EPS, 1+EPS] on both segments. Near-parallel pairs (sin of the
/// angle below ~6×10⁻⁴) return None, since they're either truly
/// parallel or below f32 precision for a meaningful answer.
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
    let len2_a = dax * dax + day * day;
    let len2_b = dbx * dbx + dby * dby;
    if denom * denom < 1e-6 * len2_a * len2_b {
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

// ---------- connected components ----------

/// Split a (vertices, edges) graph into its connected components, with
/// vertex IDs re-numbered per component. Each output pair is the
/// (vertices, edges) of one connected component of the input graph.
fn split_connected_components(
    vertices: Vec<TorusPoint>,
    edges: Vec<Edge>,
) -> Vec<(Vec<TorusPoint>, Vec<Edge>)> {
    let n = vertices.len();
    let mut adj: Vec<Vec<VertexId>> = vec![Vec::new(); n];
    for e in &edges {
        adj[e.u as usize].push(e.v);
        adj[e.v as usize].push(e.u);
    }
    // Component label per vertex.
    let mut label: Vec<i32> = vec![-1; n];
    let mut next_label: i32 = 0;
    for v in 0..n {
        if label[v] != -1 {
            continue;
        }
        // BFS from v.
        label[v] = next_label;
        let mut queue = vec![v as VertexId];
        while let Some(x) = queue.pop() {
            for &y in &adj[x as usize] {
                if label[y as usize] == -1 {
                    label[y as usize] = next_label;
                    queue.push(y);
                }
            }
        }
        next_label += 1;
    }

    // Bucket vertices and edges by component, building old→new index maps.
    let n_components = next_label as usize;
    let mut comp_vertices: Vec<Vec<TorusPoint>> = vec![Vec::new(); n_components];
    let mut comp_edges: Vec<Vec<Edge>> = vec![Vec::new(); n_components];
    let mut old_to_new: Vec<VertexId> = vec![0; n];
    for (vid, &lbl) in label.iter().enumerate() {
        let c = lbl as usize;
        old_to_new[vid] = comp_vertices[c].len() as VertexId;
        comp_vertices[c].push(vertices[vid]);
    }
    for e in edges {
        let c = label[e.u as usize] as usize;
        comp_edges[c].push(Edge {
            u: old_to_new[e.u as usize],
            v: old_to_new[e.v as usize],
            winding: e.winding,
        });
    }
    // Drop components that ended up empty of edges (isolated vertices
    // shouldn't normally happen, but be defensive).
    comp_vertices
        .into_iter()
        .zip(comp_edges.into_iter())
        .filter(|(_, e)| !e.is_empty())
        .collect()
}

// ---------- DCEL: half-edges and faces ----------

fn half_edge_disp(
    vertices: &[TorusPoint],
    edges: &[Edge],
    half_edges: &[HalfEdge],
    h: u32,
) -> (f32, f32) {
    let he = &half_edges[h as usize];
    let edge = &edges[he.edge as usize];
    let u_pos = vertices[edge.u as usize];
    let v_pos = vertices[edge.v as usize];
    if he.origin == edge.u {
        (
            u_pos.x.signed_diff_to(v_pos.x) + edge.winding.0 as f32,
            u_pos.y.signed_diff_to(v_pos.y) + edge.winding.1 as f32,
        )
    } else {
        (
            v_pos.x.signed_diff_to(u_pos.x) - edge.winding.0 as f32,
            v_pos.y.signed_diff_to(u_pos.y) - edge.winding.1 as f32,
        )
    }
}

fn build_half_edges_and_faces(
    vertices: &[TorusPoint],
    edges: &[Edge],
) -> (Vec<HalfEdge>, Vec<Face>) {
    let mut half_edges: Vec<HalfEdge> = Vec::with_capacity(edges.len() * 2);
    for (i, e) in edges.iter().enumerate() {
        let h0 = half_edges.len() as u32;
        let h1 = h0 + 1;
        half_edges.push(HalfEdge {
            origin: e.u,
            edge: i as u32,
            twin: h1,
            next: u32::MAX,
            face: u32::MAX,
        });
        half_edges.push(HalfEdge {
            origin: e.v,
            edge: i as u32,
            twin: h0,
            next: u32::MAX,
            face: u32::MAX,
        });
    }

    // Outgoing half-edges per vertex, sorted by angle of their disp.
    let mut outgoing: Vec<Vec<u32>> = vec![Vec::new(); vertices.len()];
    for (hi, he) in half_edges.iter().enumerate() {
        outgoing[he.origin as usize].push(hi as u32);
    }
    for list in outgoing.iter_mut() {
        list.sort_by(|&a, &b| {
            let (ax, ay) = half_edge_disp(vertices, edges, &half_edges, a);
            let (bx, by) = half_edge_disp(vertices, edges, &half_edges, b);
            let aa = ay.atan2(ax);
            let ab = by.atan2(bx);
            aa.partial_cmp(&ab).unwrap_or(std::cmp::Ordering::Equal)
        });
    }
    let mut out_idx: Vec<u32> = vec![0; half_edges.len()];
    for list in &outgoing {
        for (k, &h) in list.iter().enumerate() {
            out_idx[h as usize] = k as u32;
        }
    }

    // Set `next` pointers: next(h) = clockwise-of-twin at h's destination.
    for hi in 0..half_edges.len() {
        let twin = half_edges[hi].twin;
        let v = half_edges[twin as usize].origin;
        let list = &outgoing[v as usize];
        let k = out_idx[twin as usize];
        let n = list.len() as u32;
        let prev = list[((k + n - 1) % n) as usize];
        half_edges[hi].next = prev;
    }

    // Trace faces.
    let mut faces: Vec<Face> = Vec::new();
    let mut visited = vec![false; half_edges.len()];
    for start in 0..half_edges.len() {
        if visited[start] {
            continue;
        }
        let face_idx = faces.len() as u32;
        let mut cycle: Vec<u32> = Vec::new();
        let mut h = start as u32;
        let mut guard = 0;
        loop {
            if visited[h as usize] || guard > half_edges.len() * 4 {
                break;
            }
            visited[h as usize] = true;
            cycle.push(h);
            half_edges[h as usize].face = face_idx;
            h = half_edges[h as usize].next;
            if h == start as u32 {
                break;
            }
            guard += 1;
        }

        let start_v = half_edges[cycle[0] as usize].origin;
        let start_pos = vertices[start_v as usize];
        let mut polygon: Vec<(f32, f32)> = Vec::with_capacity(cycle.len());
        let mut x = start_pos.x.to_f32();
        let mut y = start_pos.y.to_f32();
        polygon.push((x, y));
        let mut total_x = 0.0;
        let mut total_y = 0.0;
        for &he in &cycle {
            let (dx, dy) = half_edge_disp(vertices, edges, &half_edges, he);
            x += dx;
            y += dy;
            total_x += dx;
            total_y += dy;
            polygon.push((x, y));
        }
        let closed = total_x.abs() < EPS && total_y.abs() < EPS;
        if closed {
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

    (half_edges, faces)
}

// ---------- geometry / rendering helpers ----------

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

fn face_color(poly: &[(f32, f32)]) -> Color {
    if poly.is_empty() {
        return Color::new(0.0, 0.0, 0.0, 0.0);
    }
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

pub fn triangulate(poly: &[(f32, f32)]) -> Vec<[(f32, f32); 3]> {
    let n = poly.len();
    if n < 3 {
        return Vec::new();
    }
    for i in 0..n {
        for j in (i + 2)..n {
            if i == 0 && j == n - 1 {
                continue;
            }
            if (poly[i].0 - poly[j].0).abs() < EPS && (poly[i].1 - poly[j].1).abs() < EPS {
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
    if polygon_signed_area(&pts) < 0.0 {
        pts.reverse();
    }
    let mut idx: Vec<usize> = (0..pts.len()).collect();
    let mut out: Vec<[(f32, f32); 3]> = Vec::with_capacity(n - 2);
    let mut guard = 0;
    while idx.len() > 3 {
        guard += 1;
        if guard > n * n + 10 {
            return out;
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
                continue;
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

// ---------- topological face count via rasterization ----------

const RASTER_GRID: usize = 256;

fn compute_topological_face_count(vertices: &[TorusPoint], edges: &[Edge]) -> usize {
    if edges.is_empty() {
        return 1;
    }
    let n = RASTER_GRID;
    let mut blocked = vec![false; n * n];
    let g = n as f32;
    for e in edges {
        let (u_pos, dx, dy) = edge_lifted_geometry(vertices, *e);
        let len = dx.abs().max(dy.abs()).max(0.001);
        let steps = ((len * g) as usize).max(2) * 3;
        for s in 0..=steps {
            let t = s as f32 / steps as f32;
            let x = (u_pos.0 + t * dx).rem_euclid(1.0);
            let y = (u_pos.1 + t * dy).rem_euclid(1.0);
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

// ---------- tests ----------

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
    fn empty_glyph() {
        let g = Glyph::from_segments(vec![]);
        assert!(g.vertices.is_empty());
        assert!(g.edges.is_empty());
        assert!(g.faces.is_empty());
    }

    #[test]
    fn crossing_segments_share_intersection_vertex() {
        let segs = vec![
            seg(0.3, 0.5, 0.7, 0.5),
            seg(0.5, 0.3, 0.5, 0.7),
        ];
        let g = Glyph::from_segments(segs);
        // 4 original endpoints + 1 intersection = 5 vertices.
        assert_eq!(g.vertices.len(), 5);
        // 2 originals, each split in half → 4 edges.
        assert_eq!(g.edges.len(), 4);
    }

    #[test]
    fn triangle_yields_inner_and_outer_faces() {
        let segs = vec![
            seg(0.4, 0.4, 0.6, 0.4),
            seg(0.6, 0.4, 0.5, 0.6),
            seg(0.5, 0.6, 0.4, 0.4),
        ];
        let g = Glyph::from_segments(segs);
        assert_eq!(g.vertices.len(), 3);
        assert_eq!(g.edges.len(), 3);
        assert_eq!(g.faces.len(), 2);
        let signs: Vec<f32> = g.faces.iter().map(|f| f.signed_area).collect();
        assert!(signs.iter().any(|&s| s > 1e-4));
        assert!(signs.iter().any(|&s| s < -1e-4));
    }

    #[test]
    fn longitude_self_loop_has_winding() {
        // A segment from (0, 0.5) wrapping to (0, 0.5) (disp = (1, 0)).
        let s = seg(0.0, 0.5, 1.0, 0.5);
        let g = Glyph::from_segments(vec![s]);
        assert_eq!(g.vertices.len(), 1);
        assert_eq!(g.edges.len(), 1);
        let e = g.edges[0];
        assert_eq!(e.u, e.v);
        assert_eq!(e.winding, (1, 0));
    }

    #[test]
    fn topological_meridian_does_not_separate_torus() {
        let s = seg(0.0, 0.5, 1.0, 0.5);
        let g = Glyph::from_segments(vec![s]);
        assert_eq!(g.topological_face_count, 1);
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
    fn partition_separates_disjoint_groups() {
        let all = vec![
            seg(0.10, 0.10, 0.20, 0.10),
            seg(0.20, 0.10, 0.15, 0.20),
            seg(0.15, 0.20, 0.10, 0.10),
            seg(0.70, 0.70, 0.80, 0.70),
            seg(0.80, 0.70, 0.75, 0.80),
            seg(0.75, 0.80, 0.70, 0.70),
        ];
        assert_eq!(partition_into_glyphs(all).len(), 2);
    }

    #[test]
    fn partition_groups_touching_segments_together() {
        let segs = vec![
            seg(0.3, 0.5, 0.7, 0.5),
            seg(0.5, 0.3, 0.5, 0.7),
        ];
        assert_eq!(partition_into_glyphs(segs).len(), 1);
    }

    #[test]
    fn focused_t_junction() {
        let seg_a = TorusSegment {
            start: TorusPoint::new(0.59636897, 0.18071648),
            disp: TorusVec::new(-0.00349844, -0.01244595),
        };
        let seg_b = TorusSegment {
            start: TorusPoint::new(0.5835103, 0.17677516),
            disp: TorusVec::new(0.01417563, -0.01287985),
        };
        let g = Glyph::from_segments(vec![seg_a, seg_b]);
        assert_eq!(g.edges.len(), 3);
        check_no_interior_crossings(&g).unwrap();
    }

    // ---------- fuzz tests ----------

    struct Lcg(u64);
    impl Lcg {
        fn new(seed: u64) -> Self {
            Self(seed.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(1))
        }
        fn next_u32(&mut self) -> u32 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 32) as u32
        }
        fn next_f32(&mut self) -> f32 {
            (self.next_u32() as f64 / u32::MAX as f64) as f32
        }
    }

    fn random_segment(rng: &mut Lcg) -> TorusSegment {
        let sx = rng.next_f32();
        let sy = rng.next_f32();
        let dx = rng.next_f32() - 0.5;
        let dy = rng.next_f32() - 0.5;
        TorusSegment {
            start: TorusPoint::new(sx, sy),
            disp: TorusVec::new(dx, dy),
        }
    }

    /// Invariant 1: no two edges of a glyph share an interior point.
    fn check_no_interior_crossings(g: &Glyph) -> Result<(), String> {
        const TOL: f32 = 0.004;
        for i in 0..g.edges.len() {
            for j in (i + 1)..g.edges.len() {
                let a = g.edge_segment(i);
                let b = g.edge_segment(j);
                let a0 = (a.start.x.to_f32(), a.start.y.to_f32());
                let a1 = (a0.0 + a.disp.dx, a0.1 + a.disp.dy);
                let b0c = (b.start.x.to_f32(), b.start.y.to_f32());
                let b1c = (b0c.0 + b.disp.dx, b0c.1 + b.disp.dy);
                for li in -1..=1 {
                    for lj in -1..=1 {
                        let ox = li as f32;
                        let oy = lj as f32;
                        let b0 = (b0c.0 + ox, b0c.1 + oy);
                        let b1 = (b1c.0 + ox, b1c.1 + oy);
                        let Some((_ta, _tb, px, py)) = seg_seg_intersect(a0, a1, b0, b1) else {
                            continue;
                        };
                        let p = TorusPoint::new(px, py);
                        let at_a =
                            p.distance_to(a.start) < TOL || p.distance_to(a.end()) < TOL;
                        let at_b =
                            p.distance_to(b.start) < TOL || p.distance_to(b.end()) < TOL;
                        if !(at_a && at_b) {
                            return Err(format!(
                                "edges {i} and {j} cross at ({px:.4}, {py:.4}) — not at an endpoint of both"
                            ));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Invariant 2: every vertex of the glyph is reachable from every
    /// other via edges (single connected component).
    fn check_connected(g: &Glyph) -> Result<(), String> {
        let n = g.vertices.len();
        if n == 0 {
            return Ok(());
        }
        let mut adj: Vec<Vec<VertexId>> = vec![Vec::new(); n];
        for e in &g.edges {
            adj[e.u as usize].push(e.v);
            adj[e.v as usize].push(e.u);
        }
        let mut visited = vec![false; n];
        visited[0] = true;
        let mut stack = vec![0u32];
        while let Some(v) = stack.pop() {
            for &y in &adj[v as usize] {
                if !visited[y as usize] {
                    visited[y as usize] = true;
                    stack.push(y);
                }
            }
        }
        let unreached: Vec<usize> = (0..n).filter(|&i| !visited[i]).collect();
        if !unreached.is_empty() {
            return Err(format!(
                "{} unreached vertices out of {}",
                unreached.len(),
                n
            ));
        }
        Ok(())
    }

    fn run_fuzz(seed: u64, iterations: usize) {
        let mut rng = Lcg::new(seed);
        let mut glyphs: Vec<Glyph> = Vec::new();
        for iter in 0..iterations {
            let action = rng.next_u32() % 100;
            let total_edges: usize = glyphs.iter().map(|g| g.edges.len()).sum();
            if action < 70 || total_edges == 0 {
                add_segment(&mut glyphs, random_segment(&mut rng));
            } else {
                let target = (rng.next_u32() as usize) % total_edges;
                let mut acc = 0;
                let mut found = None;
                for (gi, g) in glyphs.iter().enumerate() {
                    if acc + g.edges.len() > target {
                        found = Some((gi, target - acc));
                        break;
                    }
                    acc += g.edges.len();
                }
                let (gi, ei) = found.expect("uniform pick must land somewhere");
                remove_edge(&mut glyphs, gi, ei);
            }
            for (idx, g) in glyphs.iter().enumerate() {
                if let Err(e) = check_no_interior_crossings(g) {
                    panic!("seed {seed}, iter {iter}, glyph {idx}: invariant 1: {e}");
                }
                if let Err(e) = check_connected(g) {
                    panic!("seed {seed}, iter {iter}, glyph {idx}: invariant 2: {e}");
                }
            }
        }
    }

    #[test]
    fn fuzz_invariants_seed_0() {
        run_fuzz(0, 120);
    }

    #[test]
    fn fuzz_invariants_seed_1() {
        run_fuzz(1, 120);
    }

    #[test]
    fn fuzz_invariants_seed_2() {
        run_fuzz(2, 120);
    }

    /// Building the same sequence of segments — once as-is and once
    /// with every input shifted by a uniform delta — produces glyphs
    /// with the same structure. The Coord type's translation
    /// invariance, plus the lifted_disp = signed_diff_to + winding
    /// convention, are supposed to make this hold across glyph
    /// operations even when the shift drags vertices across the 0/1
    /// boundary.
    #[test]
    fn fuzz_glyph_structure_translation_invariant() {
        let mut rng = Lcg::new(99);
        for trial in 0..20 {
            let shift = TorusVec::new(rng.next_f32() * 4.0 - 2.0, rng.next_f32() * 4.0 - 2.0);
            let segs: Vec<TorusSegment> = (0..10).map(|_| random_segment(&mut rng)).collect();
            let mut glyphs_a: Vec<Glyph> = Vec::new();
            let mut glyphs_b: Vec<Glyph> = Vec::new();
            for s in &segs {
                add_segment(&mut glyphs_a, *s);
                add_segment(
                    &mut glyphs_b,
                    TorusSegment {
                        start: s.start.translate(shift),
                        disp: s.disp,
                    },
                );
            }
            assert_eq!(
                glyphs_a.len(),
                glyphs_b.len(),
                "trial {trial} (shift={:?}): glyph count differs",
                shift
            );
            for (gi, (ga, gb)) in glyphs_a.iter().zip(glyphs_b.iter()).enumerate() {
                assert_eq!(
                    ga.vertices.len(),
                    gb.vertices.len(),
                    "trial {trial} glyph {gi}: V"
                );
                assert_eq!(
                    ga.edges.len(),
                    gb.edges.len(),
                    "trial {trial} glyph {gi}: E"
                );
                assert_eq!(
                    ga.faces.len(),
                    gb.faces.len(),
                    "trial {trial} glyph {gi}: F (combinatorial)"
                );
                // topological_face_count comes from rasterization on
                // a fixed grid and is fundamentally only approximately
                // translation-invariant (cell-quantization can merge
                // or split nearby segments depending on alignment).
                // Don't assert on it here.
            }
        }
    }
}
