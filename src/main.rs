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
const HIT_RADIUS_DEFAULT_PX: f32 = 10.0;
const HIT_RADIUS_MIN_PX: f32 = 2.0;
const HIT_RADIUS_MAX_PX: f32 = 40.0;
const LINE_WIDTH: f32 = 2.5;

const SLIDER_WIDTH: f32 = 220.0;
const SLIDER_HEIGHT: f32 = 8.0;
const SLIDER_KNOB_RADIUS: f32 = 8.0;
const SLIDER_MARGIN: f32 = 20.0;

/// What the user is currently doing with the mouse.
enum Drag {
    /// Drawing a brand-new segment; we own its growing displacement.
    DrawNew { start: TorusPoint, disp: TorusVec },
    /// Translating an entire glyph by the cumulative mouse delta.
    MoveGlyph { glyph_idx: usize },
}

struct App {
    glyphs: Vec<Glyph>,
    drag: Option<Drag>,
    hit_radius_px: f32,
    slider_drag: bool,
    /// Past states, most recent on top. `undo` pops one and restores it.
    undo_stack: Vec<Vec<TorusSegment>>,
    /// States that were undone and can be re-applied.
    redo_stack: Vec<Vec<TorusSegment>>,
}

impl App {
    fn new() -> Self {
        Self {
            glyphs: Vec::new(),
            drag: None,
            hit_radius_px: HIT_RADIUS_DEFAULT_PX,
            slider_drag: false,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    /// Flatten every glyph's segments into one list. The grouping into
    /// glyphs is recovered from this list on `restore` by reconnecting
    /// touching segments.
    fn snapshot(&self) -> Vec<TorusSegment> {
        self.glyphs
            .iter()
            .flat_map(|g| g.segments.iter().copied())
            .collect()
    }

    /// Replace the current glyphs with whatever the given flat segment
    /// list implies, after partitioning into connected components.
    fn restore(&mut self, segments: Vec<TorusSegment>) {
        self.glyphs = glyph::partition_into_glyphs(segments)
            .into_iter()
            .map(Glyph::from_chopped_segments)
            .collect();
    }

    /// Snapshot the current state into the undo stack and clear the
    /// redo stack. Call this just before a state-changing action.
    fn commit_action(&mut self) {
        self.undo_stack.push(self.snapshot());
        self.redo_stack.clear();
    }

    fn undo(&mut self) {
        if let Some(prev) = self.undo_stack.pop() {
            let current = self.snapshot();
            self.redo_stack.push(current);
            self.restore(prev);
        }
    }

    fn redo(&mut self) {
        if let Some(next) = self.redo_stack.pop() {
            let current = self.snapshot();
            self.undo_stack.push(current);
            self.restore(next);
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

    fn place_new_segment(&mut self, seg: TorusSegment) {
        glyph::add_segment(&mut self.glyphs, seg);
    }

    fn rebuild_glyph(&mut self, i: usize) {
        // Drag-time rebuild: don't re-chop the segments (that would
        // reorder them and invalidate drag indices). Just rebuild the
        // DCEL on the current segments.
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
        draw_slider(&app);

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
    let just_released_left = is_mouse_button_released(MouseButton::Left);

    // Slider takes priority over canvas interactions.
    let (slider_pos, slider_size) = slider_rect();
    if just_pressed_left && point_in_slider(mouse_px, slider_pos, slider_size) {
        app.slider_drag = true;
    }
    if app.slider_drag {
        let t = ((mouse_px.x - slider_pos.x) / slider_size.x).clamp(0.0, 1.0);
        app.hit_radius_px = HIT_RADIUS_MIN_PX + t * (HIT_RADIUS_MAX_PX - HIT_RADIUS_MIN_PX);
        if just_released_left {
            app.slider_drag = false;
        }
        return;
    }

    let snap_radius = app.hit_radius_px / canvas_size;

    // Keyboard shortcuts: Ctrl/Cmd+Z = undo, Ctrl/Cmd+Shift+Z or
    // Ctrl/Cmd+Y = redo.
    let ctrl = is_key_down(KeyCode::LeftControl)
        || is_key_down(KeyCode::RightControl)
        || is_key_down(KeyCode::LeftSuper)
        || is_key_down(KeyCode::RightSuper);
    let shift = is_key_down(KeyCode::LeftShift) || is_key_down(KeyCode::RightShift);
    if ctrl && is_key_pressed(KeyCode::Z) {
        if shift {
            app.redo();
        } else {
            app.undo();
        }
        return;
    }
    if ctrl && is_key_pressed(KeyCode::Y) {
        app.redo();
        return;
    }

    // Right-click: delete the segment under the cursor (if any). If
    // removing it disconnects the glyph, the remainder is re-
    // partitioned into multiple glyphs.
    if is_mouse_button_pressed(MouseButton::Right) && in_canvas {
        if let Some((gi, si)) = pick_segment_in_glyphs(&app.glyphs, mouse_torus) {
            app.commit_action();
            glyph::remove_segment(&mut app.glyphs, gi, si);
        }
        return;
    }

    if just_pressed_left && in_canvas {
        if let Some((gi, _si)) = pick_segment_in_glyphs(&app.glyphs, mouse_torus) {
            // Click on any part of a glyph drags the whole glyph.
            app.commit_action();
            app.drag = Some(Drag::MoveGlyph { glyph_idx: gi });
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
                Drag::MoveGlyph { glyph_idx } => {
                    let gi = *glyph_idx;
                    for s in &mut app.glyphs[gi].segments {
                        *s = s.translated(mouse_delta);
                    }
                    // Rebuild the affected glyph's DCEL so face overlays
                    // follow the drag.
                    app.rebuild_glyph(gi);
                }
            }
        }
    }

    if is_mouse_button_released(MouseButton::Left) {
        match app.drag.take() {
            Some(Drag::DrawNew { start, disp }) => {
                // Discard zero-length scratches.
                if disp.length() <= 0.005 {
                    return;
                }
                // Snap the released endpoint to a nearby vertex.
                let raw_end = start.translate(disp);
                let snapped_end = app
                    .snap_target(raw_end, app.hit_radius_px / canvas_size)
                    .unwrap_or(raw_end);
                let final_disp = TorusVec::new(
                    disp.dx + (snapped_end.x() - raw_end.x()),
                    disp.dy + (snapped_end.y() - raw_end.y()),
                );
                // The correction above can be off by ±1 if raw_end was
                // near the seam and snapped_end wrapped. Normalize by
                // picking the equivalent (mod 1) shift with the
                // smallest magnitude.
                let final_disp = TorusVec::new(
                    normalize_disp_delta(final_disp.dx, disp.dx),
                    normalize_disp_delta(final_disp.dy, disp.dy),
                );
                app.commit_action();
                app.place_new_segment(TorusSegment {
                    start,
                    disp: final_disp,
                });
            }
            Some(Drag::MoveGlyph { .. }) => {
                // commit_action was called when the drag started; no
                // additional snapshot needed on release.
            }
            None => {}
        }
    }
}

/// Position and size of the hit-radius slider track, in screen pixels.
fn slider_rect() -> (Vec2, Vec2) {
    let w = screen_width();
    let x = w - SLIDER_WIDTH - SLIDER_MARGIN;
    let y = SLIDER_MARGIN;
    (vec2(x, y), vec2(SLIDER_WIDTH, SLIDER_HEIGHT))
}

fn point_in_slider(p: Vec2, track_pos: Vec2, track_size: Vec2) -> bool {
    let pad = SLIDER_KNOB_RADIUS + 2.0;
    p.x >= track_pos.x - pad
        && p.x <= track_pos.x + track_size.x + pad
        && p.y >= track_pos.y - pad
        && p.y <= track_pos.y + track_size.y + pad
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

fn draw_slider(app: &App) {
    let (pos, size) = slider_rect();
    draw_rectangle(pos.x, pos.y, size.x, size.y, Color::from_rgba(40, 40, 50, 255));
    draw_rectangle_lines(
        pos.x,
        pos.y,
        size.x,
        size.y,
        1.0,
        Color::from_rgba(90, 90, 110, 255),
    );
    let t = ((app.hit_radius_px - HIT_RADIUS_MIN_PX)
        / (HIT_RADIUS_MAX_PX - HIT_RADIUS_MIN_PX))
        .clamp(0.0, 1.0);
    let knob_x = pos.x + t * size.x;
    let knob_y = pos.y + size.y * 0.5;
    draw_circle(
        knob_x,
        knob_y,
        SLIDER_KNOB_RADIUS,
        Color::from_rgba(220, 200, 90, 255),
    );
    draw_circle_lines(
        knob_x,
        knob_y,
        SLIDER_KNOB_RADIUS,
        1.5,
        Color::from_rgba(20, 20, 30, 255),
    );
    let label = format!("hit radius: {:>4.1}px", app.hit_radius_px);
    draw_text(
        &label,
        pos.x,
        pos.y + size.y + 18.0,
        16.0,
        Color::from_rgba(180, 180, 200, 255),
    );
}

fn draw_hud(app: &App) {
    let n_glyphs = app.glyphs.len();
    let n_segs: usize = app.glyphs.iter().map(|g| g.segments.len()).sum();
    let n_faces: usize = app.glyphs.iter().map(|g| g.topological_face_count).sum();
    let undo_n = app.undo_stack.len();
    let redo_n = app.redo_stack.len();
    let summary = format!(
        "glyphs: {n_glyphs}  segments: {n_segs}  faces: {n_faces}    left-drag: draw / move    right-click: delete    ctrl-z: undo ({undo_n})  ctrl-shift-z: redo ({redo_n})"
    );
    draw_text(
        &summary,
        12.0,
        22.0,
        18.0,
        Color::from_rgba(180, 180, 200, 255),
    );
    for (i, g) in app.glyphs.iter().enumerate() {
        let v = g.dcel.vertices.len();
        let e = g.dcel.half_edges.len() / 2;
        let f = g.topological_face_count;
        let s = g.segments.len();
        let line = format!("  glyph {i}: V={v}  E={e}  F={f}  segments={s}");
        let y = 22.0 + 18.0 * (i + 1) as f32;
        draw_text(&line, 12.0, y, 16.0, Color::from_rgba(160, 160, 180, 255));
    }
}
