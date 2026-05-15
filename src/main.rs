//! Glyph Game — MVP drawing surface on the flat torus.
//!
//! Click and drag on empty space to draw a new line segment. Click on an
//! existing endpoint (small circle) to drag it; click on a segment's
//! interior to drag the whole segment. Right-click on a segment to delete
//! it. The viewport is a square fundamental domain of the torus, and
//! anything that leaves one edge reappears on the opposite edge.

use macroquad::prelude::*;

mod glyph;
mod torus;
use glyph::{segment_touches_glyph, triangulate, Glyph};
use torus::{TorusPoint, TorusSegment, TorusVec};

const ENDPOINT_RADIUS: f32 = 6.0;
const HIT_RADIUS_PX: f32 = 10.0;
const LINE_WIDTH: f32 = 2.5;

/// What the user is currently doing with the mouse.
enum Drag {
    /// Drawing a brand-new segment; we own its growing displacement.
    /// `glyph` is `None` because new segments don't belong to a glyph
    /// until released (and possibly merged).
    DrawNew { start: TorusPoint, disp: TorusVec },
    /// Modifying segment `seg_idx` inside `glyph_idx`. Dragging doesn't
    /// change glyph membership (per design), but it does change the
    /// underlying segments so the affected glyph's DCEL is rebuilt on
    /// every frame the drag advances.
    Modify {
        glyph_idx: usize,
        seg_idx: usize,
        kind: ModifyKind,
    },
}

#[derive(Copy, Clone)]
enum ModifyKind {
    Start,
    End,
    Whole,
}

struct App {
    glyphs: Vec<Glyph>,
    drag: Option<Drag>,
}

impl App {
    fn new() -> Self {
        Self {
            glyphs: Vec::new(),
            drag: None,
        }
    }

    /// Return the canonical position of the nearest vertex within
    /// `radius` of `p` across all glyphs, if any.
    fn snap_target(&self, p: TorusPoint, radius: f32) -> Option<TorusPoint> {
        let mut best: Option<(f32, TorusPoint)> = None;
        for g in &self.glyphs {
            for v in &g.dcel.vertices {
                let d = p.distance_to(*v);
                if d <= radius && best.map_or(true, |(bd, _)| d < bd) {
                    best = Some((d, *v));
                }
            }
        }
        best.map(|(_, v)| v)
    }

    /// Take a freshly-drawn segment and either start a new glyph or
    /// merge it into every existing glyph it touches.
    fn place_new_segment(&mut self, seg: TorusSegment) {
        let mut touched: Vec<usize> = (0..self.glyphs.len())
            .filter(|&i| segment_touches_glyph(&seg, &self.glyphs[i]))
            .collect();
        if touched.is_empty() {
            self.glyphs.push(Glyph::from_segments(vec![seg]));
            return;
        }
        let mut combined: Vec<TorusSegment> = vec![seg];
        // Drain touched glyphs in reverse so removals don't invalidate
        // earlier indices.
        touched.sort();
        for &i in touched.iter().rev() {
            let g = self.glyphs.remove(i);
            combined.extend(g.segments);
        }
        self.glyphs.push(Glyph::from_segments(combined));
    }

    fn rebuild_glyph(&mut self, i: usize) {
        let segs = std::mem::take(&mut self.glyphs[i].segments);
        self.glyphs[i] = Glyph::from_segments(segs);
    }
}

fn window_conf() -> Conf {
    Conf {
        window_title: "Glyph Game".to_string(),
        window_width: 900,
        window_height: 900,
        high_dpi: true,
        ..Default::default()
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    let mut app = App::new();
    // Track the previous mouse position in *screen pixels* across the
    // frame boundary so we can compute exact deltas (in pixels) and
    // convert into torus space. This avoids any seam-crossing weirdness:
    // the cursor lives in the screen, which is just ℝ².
    let mut prev_mouse_px: Option<Vec2> = None;

    loop {
        let (canvas_origin, canvas_size) = canvas_rect();
        let mouse_px = vec2(mouse_position().0, mouse_position().1);
        let mouse_torus = screen_to_torus(mouse_px, canvas_origin, canvas_size);
        let in_canvas = point_in_canvas(mouse_px, canvas_origin, canvas_size);

        handle_input(
            &mut app,
            mouse_px,
            mouse_torus,
            in_canvas,
            prev_mouse_px,
            canvas_size,
        );

        clear_background(Color::from_rgba(18, 18, 22, 255));
        draw_canvas_frame(canvas_origin, canvas_size);
        draw_scene(&app, canvas_origin, canvas_size);
        draw_hud(&app);

        prev_mouse_px = Some(mouse_px);
        next_frame().await;
    }
}

/// The square fundamental-domain viewport, centered in the window.
fn canvas_rect() -> (Vec2, f32) {
    let w = screen_width();
    let h = screen_height();
    let size = w.min(h) * 0.92;
    let origin = vec2((w - size) / 2.0, (h - size) / 2.0);
    (origin, size)
}

fn screen_to_torus(p: Vec2, origin: Vec2, size: f32) -> TorusPoint {
    TorusPoint::new((p.x - origin.x) / size, (p.y - origin.y) / size)
}

fn point_in_canvas(p: Vec2, origin: Vec2, size: f32) -> bool {
    p.x >= origin.x && p.x <= origin.x + size && p.y >= origin.y && p.y <= origin.y + size
}

fn handle_input(
    app: &mut App,
    mouse_px: Vec2,
    mouse_torus: TorusPoint,
    in_canvas: bool,
    prev_mouse_px: Option<Vec2>,
    canvas_size: f32,
) {
    let just_pressed_left = is_mouse_button_pressed(MouseButton::Left);
    let snap_radius = HIT_RADIUS_PX / canvas_size;

    // Right-click: delete the segment under the cursor (if any).
    if is_mouse_button_pressed(MouseButton::Right) && in_canvas {
        if let Some((gi, si)) = pick_segment_in_glyphs(&app.glyphs, mouse_torus) {
            app.glyphs[gi].segments.remove(si);
            if app.glyphs[gi].segments.is_empty() {
                app.glyphs.remove(gi);
            } else {
                app.rebuild_glyph(gi);
            }
        }
        return;
    }

    if just_pressed_left && in_canvas {
        // Endpoint hit takes priority over segment-body hit.
        if let Some((gi, si, kind)) = pick_endpoint(&app.glyphs, mouse_torus, snap_radius) {
            app.drag = Some(Drag::Modify {
                glyph_idx: gi,
                seg_idx: si,
                kind,
            });
        } else if let Some((gi, si)) = pick_segment_in_glyphs(&app.glyphs, mouse_torus) {
            app.drag = Some(Drag::Modify {
                glyph_idx: gi,
                seg_idx: si,
                kind: ModifyKind::Whole,
            });
        } else {
            // Snap the start point to a nearby vertex if there is one.
            let start = app.snap_target(mouse_torus, snap_radius).unwrap_or(mouse_torus);
            app.drag = Some(Drag::DrawNew {
                start,
                disp: TorusVec::zero(),
            });
        }
    }

    // Mouse delta in torus coords for this frame. We use the screen-pixel
    // delta (which never wraps) and rescale — so dragging across the
    // torus seam works without snapping. We skip the delta on the frame
    // the button was pressed, because it would reflect motion from before
    // the drag began.
    if let (Some(prev), Some(drag)) = (prev_mouse_px, app.drag.as_mut()) {
        if !just_pressed_left {
            let mouse_delta = TorusVec::new(
                (mouse_px.x - prev.x) / canvas_size,
                (mouse_px.y - prev.y) / canvas_size,
            );
            match drag {
                Drag::DrawNew { disp, .. } => {
                    *disp = *disp + mouse_delta;
                }
                Drag::Modify {
                    glyph_idx,
                    seg_idx,
                    kind,
                } => {
                    let seg = &mut app.glyphs[*glyph_idx].segments[*seg_idx];
                    *seg = match kind {
                        ModifyKind::Start => seg.move_start(mouse_delta),
                        ModifyKind::End => seg.move_end(mouse_delta),
                        ModifyKind::Whole => seg.translated(mouse_delta),
                    };
                    // Rebuild the affected glyph's DCEL so face overlays
                    // follow the drag.
                    let gi = *glyph_idx;
                    app.rebuild_glyph(gi);
                }
            }
        }
    }

    if is_mouse_button_released(MouseButton::Left) {
        if let Some(Drag::DrawNew { start, disp }) = app.drag.take() {
            // Discard zero-length scratches.
            if disp.length() <= 0.005 {
                return;
            }
            // Snap the released endpoint to a nearby vertex (other than
            // a vertex that's effectively where we already are).
            let raw_end = start.translate(disp);
            let snapped_end = app
                .snap_target(raw_end, HIT_RADIUS_PX / canvas_size)
                .unwrap_or(raw_end);
            // Update disp so the segment ends exactly on the snapped vertex
            // *along the same path the user was drawing* (preserve the lift
            // implied by `disp`).
            let final_disp = TorusVec::new(
                disp.dx + (snapped_end.x() - raw_end.x()),
                disp.dy + (snapped_end.y() - raw_end.y()),
            );
            // The end-x correction above can be off by ±1 if raw_end was
            // near the seam and snapped_end wrapped. Normalize by picking
            // the equivalent (mod 1) shift with the smallest magnitude.
            let final_disp = TorusVec::new(
                normalize_disp_delta(final_disp.dx, disp.dx),
                normalize_disp_delta(final_disp.dy, disp.dy),
            );
            app.place_new_segment(TorusSegment {
                start,
                disp: final_disp,
            });
        } else {
            app.drag = None;
        }
    }
}

/// Pick the equivalent (mod 1) value of `candidate` that's closest to
/// `reference`. Used to keep a snapped-endpoint correction from
/// changing the segment's homotopy class.
fn normalize_disp_delta(candidate: f32, reference: f32) -> f32 {
    let mut best = candidate;
    for k in -1..=1 {
        let v = candidate + k as f32;
        if (v - reference).abs() < (best - reference).abs() {
            best = v;
        }
    }
    best
}

/// Hit-test endpoints across all glyphs.
fn pick_endpoint(
    glyphs: &[Glyph],
    p: TorusPoint,
    radius: f32,
) -> Option<(usize, usize, ModifyKind)> {
    let mut best: Option<(f32, usize, usize, ModifyKind)> = None;
    for (gi, g) in glyphs.iter().enumerate() {
        for (si, seg) in g.segments.iter().enumerate() {
            for (kind, ep) in [
                (ModifyKind::Start, seg.start),
                (ModifyKind::End, seg.end()),
            ] {
                let d = p.distance_to(ep);
                if d <= radius && best.map_or(true, |(bd, _, _, _)| d < bd) {
                    best = Some((d, gi, si, kind));
                }
            }
        }
    }
    best.map(|(_, gi, si, k)| (gi, si, k))
}

/// Hit-test segments across all glyphs. Returns (glyph_idx, seg_idx).
fn pick_segment_in_glyphs(glyphs: &[Glyph], p: TorusPoint) -> Option<(usize, usize)> {
    let hit = 0.012_f32; // ~1.2% of viewport
    let mut best: Option<(f32, usize, usize)> = None;
    for (gi, g) in glyphs.iter().enumerate() {
        for (si, seg) in g.segments.iter().enumerate() {
            let d = distance_point_to_segment(seg, p);
            if d <= hit && best.map_or(true, |(bd, _, _)| d < bd) {
                best = Some((d, gi, si));
            }
        }
    }
    best.map(|(_, gi, si)| (gi, si))
}

/// Shortest distance from a torus point `p` to a torus segment, computed
/// by minimizing over all visible lifts of the segment in the universal
/// cover (with `p` lifted to its canonical representative).
fn distance_point_to_segment(seg: &TorusSegment, p: TorusPoint) -> f32 {
    let px = p.x();
    let py = p.y();
    let mut best = f32::INFINITY;
    for ((ax, ay), (bx, by)) in seg.visible_lifts() {
        let d = dist_point_to_seg_2d(px, py, ax, ay, bx, by);
        if d < best {
            best = d;
        }
    }
    best
}

fn dist_point_to_seg_2d(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let dx = bx - ax;
    let dy = by - ay;
    let len2 = dx * dx + dy * dy;
    let t = if len2 < 1e-12 {
        0.0
    } else {
        (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0)
    };
    let cx = ax + t * dx;
    let cy = ay + t * dy;
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()
}

// ---------- rendering ----------

fn draw_canvas_frame(origin: Vec2, size: f32) {
    draw_rectangle(origin.x, origin.y, size, size, Color::from_rgba(8, 8, 12, 255));
    draw_rectangle_lines(
        origin.x,
        origin.y,
        size,
        size,
        2.0,
        Color::from_rgba(80, 80, 100, 255),
    );
}

fn draw_scene(app: &App, origin: Vec2, size: f32) {
    let rect_min = origin;
    let rect_max = origin + vec2(size, size);

    // Pass 1: outer faces (negative signed area). For each, tint the
    // entire canvas with its color. Inner faces drawn next overwrite
    // the parts that should have a different color.
    for g in &app.glyphs {
        for f in &g.dcel.faces {
            if f.polygon.is_empty() || f.signed_area >= 0.0 {
                continue;
            }
            draw_rectangle(origin.x, origin.y, size, size, f.color);
        }
    }
    // Pass 2: inner faces (positive signed area), filled as tiled
    // polygons.
    for g in &app.glyphs {
        for f in &g.dcel.faces {
            if f.polygon.is_empty() || f.signed_area <= 0.0 {
                continue;
            }
            draw_face_tiled(&f.polygon, f.color, origin, size, rect_min, rect_max);
        }
    }

    // Pass 3: the segments themselves.
    let mut to_draw: Vec<TorusSegment> = Vec::new();
    for g in &app.glyphs {
        to_draw.extend(g.segments.iter().copied());
    }
    if let Some(Drag::DrawNew { start, disp }) = &app.drag {
        to_draw.push(TorusSegment {
            start: *start,
            disp: *disp,
        });
    }

    for seg in &to_draw {
        draw_segment_tiled(seg, origin, size, rect_min, rect_max);
    }
    for seg in &to_draw {
        draw_endpoint_tiled(seg.start, origin, size, rect_min, rect_max, ENDPOINT_COLOR);
        draw_endpoint_tiled(seg.end(), origin, size, rect_min, rect_max, ENDPOINT_COLOR);
    }
}

const ENDPOINT_COLOR: Color = Color::new(0.9, 0.85, 0.3, 1.0);
const LINE_COLOR: Color = Color::new(0.85, 0.9, 1.0, 1.0);

fn draw_segment_tiled(seg: &TorusSegment, origin: Vec2, size: f32, rect_min: Vec2, rect_max: Vec2) {
    for ((ax, ay), (bx, by)) in seg.visible_lifts() {
        let pa = origin + vec2(ax, ay) * size;
        let pb = origin + vec2(bx, by) * size;
        if let Some((c0, c1)) = clip_line_to_rect(pa, pb, rect_min, rect_max) {
            draw_line(c0.x, c0.y, c1.x, c1.y, LINE_WIDTH, LINE_COLOR);
        }
    }
}

fn draw_endpoint_tiled(
    p: TorusPoint,
    origin: Vec2,
    size: f32,
    rect_min: Vec2,
    rect_max: Vec2,
    color: Color,
) {
    for i in -1..=1 {
        for j in -1..=1 {
            let sx = origin.x + (p.x() + i as f32) * size;
            let sy = origin.y + (p.y() + j as f32) * size;
            // Don't draw far-away copies.
            if sx + ENDPOINT_RADIUS < rect_min.x || sx - ENDPOINT_RADIUS > rect_max.x {
                continue;
            }
            if sy + ENDPOINT_RADIUS < rect_min.y || sy - ENDPOINT_RADIUS > rect_max.y {
                continue;
            }
            draw_circle(sx, sy, ENDPOINT_RADIUS, color);
            draw_circle_lines(sx, sy, ENDPOINT_RADIUS, 1.5, Color::from_rgba(20, 20, 30, 255));
        }
    }
}

/// Fill a face polygon (in torus [0,1)² coords) by triangulating and
/// drawing every integer translate that overlaps the canvas.
fn draw_face_tiled(
    polygon: &[(f32, f32)],
    color: Color,
    origin: Vec2,
    size: f32,
    rect_min: Vec2,
    rect_max: Vec2,
) {
    let tris = triangulate(polygon);
    if tris.is_empty() {
        return;
    }
    // Polygon's bounding box (in torus coords).
    let (mut min_x, mut min_y) = (f32::INFINITY, f32::INFINITY);
    let (mut max_x, mut max_y) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
    for &(x, y) in polygon {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    // We need every integer offset (ox, oy) such that the bbox + (ox, oy)
    // overlaps [0, 1]. ox ∈ ⌈-max_x⌉..=⌊1-min_x⌋ etc.
    let ox_lo = (-max_x).ceil() as i32;
    let ox_hi = (1.0 - min_x).floor() as i32;
    let oy_lo = (-max_y).ceil() as i32;
    let oy_hi = (1.0 - min_y).floor() as i32;
    // Clamp the tile loop to a sane range so a malformed polygon can't
    // blow up the render loop.
    let ox_lo = ox_lo.max(-3);
    let ox_hi = ox_hi.min(3);
    let oy_lo = oy_lo.max(-3);
    let oy_hi = oy_hi.min(3);

    for oi in ox_lo..=ox_hi {
        for oj in oy_lo..=oy_hi {
            let ox = oi as f32;
            let oy = oj as f32;
            for tri in &tris {
                let v0 = vec2(
                    origin.x + (tri[0].0 + ox) * size,
                    origin.y + (tri[0].1 + oy) * size,
                );
                let v1 = vec2(
                    origin.x + (tri[1].0 + ox) * size,
                    origin.y + (tri[1].1 + oy) * size,
                );
                let v2 = vec2(
                    origin.x + (tri[2].0 + ox) * size,
                    origin.y + (tri[2].1 + oy) * size,
                );
                // Rough triangle-vs-canvas reject.
                let lo_x = v0.x.min(v1.x).min(v2.x);
                let hi_x = v0.x.max(v1.x).max(v2.x);
                let lo_y = v0.y.min(v1.y).min(v2.y);
                let hi_y = v0.y.max(v1.y).max(v2.y);
                if hi_x < rect_min.x || lo_x > rect_max.x || hi_y < rect_min.y || lo_y > rect_max.y
                {
                    continue;
                }
                draw_triangle(v0, v1, v2, color);
            }
        }
    }
}

/// Liang–Barsky line clipping against an axis-aligned rectangle.
fn clip_line_to_rect(p0: Vec2, p1: Vec2, rmin: Vec2, rmax: Vec2) -> Option<(Vec2, Vec2)> {
    let d = p1 - p0;
    let mut t_enter = 0.0_f32;
    let mut t_exit = 1.0_f32;
    let edges = [
        (d.x, rmin.x - p0.x, rmax.x - p0.x),
        (d.y, rmin.y - p0.y, rmax.y - p0.y),
    ];
    for (delta, q_min, q_max) in edges {
        if delta.abs() < 1e-9 {
            if q_min > 0.0 || q_max < 0.0 {
                return None;
            }
        } else {
            let t1 = q_min / delta;
            let t2 = q_max / delta;
            let (t_in, t_out) = if t1 < t2 { (t1, t2) } else { (t2, t1) };
            t_enter = t_enter.max(t_in);
            t_exit = t_exit.min(t_out);
            if t_enter > t_exit {
                return None;
            }
        }
    }
    Some((p0 + d * t_enter, p0 + d * t_exit))
}

fn draw_hud(app: &App) {
    let n_glyphs = app.glyphs.len();
    let n_segs: usize = app.glyphs.iter().map(|g| g.segments.len()).sum();
    let n_faces: usize = app
        .glyphs
        .iter()
        .map(|g| g.dcel.faces.iter().filter(|f| !f.polygon.is_empty()).count())
        .sum();
    let line = format!(
        "glyphs: {n_glyphs}  segments: {n_segs}  faces: {n_faces}    left-drag: draw / move    right-click: delete"
    );
    draw_text(&line, 12.0, 22.0, 18.0, Color::from_rgba(180, 180, 200, 255));
}
