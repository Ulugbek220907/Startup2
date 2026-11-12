//! The design editor: text, images, freehand drawing, shapes, stickers and
//! templates on a zoomed view of the garment's print area.

use design::display::{self, DrawOp, Paint};
use design::fonts::FontId;
use design::garment_art::contrast_ink;
use design::*;
use domain::api::{AssetDto, DesignDto, GarmentDto, PrintAreaDto};
use domain::pricing::{self, PriceTable, QuoteInput};
use domain::Tiyin;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{HtmlCanvasElement, PointerEvent};

use crate::app::TopBar;
use crate::draw::{self, MockupOpts, View};
use crate::icons;
use crate::router::{use_router, Route};
use crate::state::{self, ctx, ensure_doc_fonts, ensure_font, new_layer_id, t, tf, Ctx, Fonts};
use crate::{api, platform};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    None,
    Draw,
    Shapes,
    Stickers,
    Templates,
}

#[derive(Clone, Copy, PartialEq)]
enum DragKind {
    Move,
    Scale,
    Rotate,
}

#[derive(Clone)]
struct Drag {
    kind: DragKind,
    /// Pointer position at the start, in print-area cm.
    start: (f32, f32),
    orig: Layer,
}

#[derive(Default)]
struct History {
    undo: Vec<DesignDoc>,
    redo: Vec<DesignDoc>,
    /// The last committed state: what "undo" returns to.
    last: Option<DesignDoc>,
}

const PALETTE: [&str; 10] = ["#ffffff", "#111111", "#ffdf00", "#ef4444", "#f97316", "#22c55e", "#06b6d4", "#3b82f6", "#8b5cf6", "#ec4899"];
const HANDLE_RADIUS: f32 = 24.0;
const ROTATE_OFFSET: f32 = 34.0;
const SNAP_CM: f32 = 0.3;

/// Everything the editor's closures share. All fields are `Copy` handles.
#[derive(Clone, Copy)]
struct Ed {
    ctx: Ctx,
    selected: RwSignal<Option<String>>,
    tool: RwSignal<Tool>,
    history: StoredValue<History>,
    /// `(can undo, can redo)` for the toolbar buttons.
    hist_state: RwSignal<(bool, bool)>,
    drag: StoredValue<Option<Drag>>,
    /// Stroke being drawn right now (absolute cm), shown as a preview.
    live_stroke: RwSignal<Option<Stroke>>,
    /// Layer collecting this drawing session's strokes.
    draw_layer: StoredValue<Option<String>>,
    brush_color: RwSignal<Color>,
    brush_cm: RwSignal<f32>,
    busy: RwSignal<bool>,
}

impl Ed {
    fn area(&self) -> Option<PrintAreaDto> {
        let side = self.ctx.side.get();
        self.ctx.garment().and_then(|g| g.area(side).copied())
    }

    fn area_untracked(&self) -> Option<PrintAreaDto> {
        let side = self.ctx.side.get_untracked();
        let slug = self.ctx.draft.with_untracked(|d| d.doc.as_ref().map(|doc| doc.garment.clone()))?;
        self.ctx.catalog.with_untracked(|c| c.as_ref()?.garments.iter().find(|g| g.slug == slug)?.area(side).copied())
    }

    fn doc(&self) -> Option<DesignDoc> {
        self.ctx.draft.with_untracked(|d| d.doc.clone())
    }

    /// Change the document without recording an undo step (used while a
    /// drag is in progress; `commit` records it at the end).
    fn mutate(&self, f: impl FnOnce(&mut DesignDoc)) {
        self.ctx.draft.update(|d| {
            if let Some(doc) = d.doc.as_mut() {
                f(doc);
            }
        });
    }

    fn mutate_selected(&self, f: impl FnOnce(&mut Layer)) {
        // `try_` variants: this can run from an async task after the editor
        // was closed, when its signals no longer exist.
        let Some(id) = self.selected.try_get_untracked().flatten() else { return };
        let side = self.ctx.side.get_untracked();
        self.mutate(|doc| {
            if let Some(l) = doc.layers_mut(side).iter_mut().find(|l| l.id == id) {
                f(l);
            }
        });
    }

    /// Record the current document as an undo step and save the draft.
    fn commit(&self) {
        let Some(cur) = self.doc() else { return };
        let _ = self.history.try_update_value(|h| {
            if h.last.as_ref() == Some(&cur) {
                return;
            }
            if let Some(prev) = h.last.replace(cur.clone()) {
                h.undo.push(prev);
                // Bound memory: 60 steps is plenty on a phone.
                if h.undo.len() > 60 {
                    h.undo.remove(0);
                }
            }
            h.redo.clear();
        });
        self.sync_hist();
        self.ctx.persist_draft();
        ensure_doc_fonts(&cur);
    }

    fn sync_hist(&self) {
        if let Some(state) = self.history.try_with_value(|h| (!h.undo.is_empty(), !h.redo.is_empty())) {
            self.hist_state.set(state);
        }
    }

    fn undo(&self) {
        let mut restored = None;
        self.history.update_value(|h| {
            if let (Some(prev), Some(cur)) = (h.undo.pop(), h.last.clone()) {
                h.redo.push(cur);
                h.last = Some(prev.clone());
                restored = Some(prev);
            }
        });
        self.restore(restored);
    }

    fn redo(&self) {
        let mut restored = None;
        self.history.update_value(|h| {
            if let (Some(next), Some(cur)) = (h.redo.pop(), h.last.clone()) {
                h.undo.push(cur);
                h.last = Some(next.clone());
                restored = Some(next);
            }
        });
        self.restore(restored);
    }

    fn restore(&self, doc: Option<DesignDoc>) {
        if let Some(doc) = doc {
            self.selected.set(None);
            self.ctx.set_doc(doc);
            self.sync_hist();
            platform::haptic("light");
        }
    }

    /// Add a layer on the current side, select it and commit.
    fn add_layer(&self, kind: LayerKind, x: f32, y: f32) {
        let layer = Layer::new(new_layer_id(), x, y, kind);
        let id = layer.id.clone();
        let side = self.ctx.side.get_untracked();
        self.mutate(|doc| doc.layers_mut(side).push(layer));
        self.selected.set(Some(id));
        self.commit();
        platform::haptic("light");
    }

    /// Leaving the draw tool: centre the drawing layer on its own strokes so
    /// it rotates and scales around its middle like every other layer.
    fn finish_drawing(&self) {
        let Some(id) = self.draw_layer.get_value() else { return };
        self.draw_layer.set_value(None);
        let side = self.ctx.side.get_untracked();
        self.mutate(|doc| {
            if let Some(l) = doc.layers_mut(side).iter_mut().find(|l| l.id == id) {
                let b = display::local_bounds(l, &Fonts);
                let (cx, cy) = b.center();
                if let LayerKind::Draw(d) = &mut l.kind {
                    for s in &mut d.strokes {
                        for p in &mut s.points {
                            p[0] -= cx;
                            p[1] -= cy;
                        }
                    }
                }
                l.x += cx;
                l.y += cy;
            }
        });
        self.commit();
    }

    fn set_tool(&self, tool: Tool) {
        if self.tool.get_untracked() == Tool::Draw && tool != Tool::Draw {
            self.finish_drawing();
        }
        if tool == Tool::Draw {
            self.selected.set(None);
        }
        self.tool.set(tool);
    }
}

/// Where the selection handles are, in canvas CSS pixels.
struct Handles {
    corners: [(f32, f32); 4],
    scale: (f32, f32),
    rotate: (f32, f32),
}

fn handles(layer: &Layer, base: &Affine) -> Handles {
    let b = display::local_bounds(layer, &Fonts);
    let t = display::layer_transform(layer).then(*base);
    let corners = [t.apply(b.x, b.y), t.apply(b.right(), b.y), t.apply(b.right(), b.bottom()), t.apply(b.x, b.bottom())];
    let top_mid = ((corners[0].0 + corners[1].0) / 2.0, (corners[0].1 + corners[1].1) / 2.0);
    let centre = t.apply(b.x + b.w / 2.0, b.y + b.h / 2.0);
    // Push the rotate handle outwards from the centre through the top edge.
    let (dx, dy) = (top_mid.0 - centre.0, top_mid.1 - centre.1);
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    let rotate = (top_mid.0 + dx / len * ROTATE_OFFSET, top_mid.1 + dy / len * ROTATE_OFFSET);
    Handles { corners, scale: corners[2], rotate }
}

fn near(a: (f32, f32), b: (f32, f32), r: f32) -> bool {
    (a.0 - b.0).powi(2) + (a.1 - b.1).powi(2) <= r * r
}

/// Snap a moved layer to the area's centre lines and edges.
fn snap_move(layer: &mut Layer, area: &PrintAreaDto) {
    let b = display::layer_bbox(layer, &Fonts);
    let (cx, cy) = b.center();
    let (w, h) = (area.width_cm, area.height_cm);
    // (current value, target) pairs for x, then y; first match wins.
    for (cur, target) in [(cx, w / 2.0), (b.x, 0.0), (b.right(), w)] {
        if (cur - target).abs() < SNAP_CM {
            layer.x += target - cur;
            break;
        }
    }
    for (cur, target) in [(cy, h / 2.0), (b.y, 0.0), (b.bottom(), h)] {
        if (cur - target).abs() < SNAP_CM {
            layer.y += target - cur;
            break;
        }
    }
    // Never let a layer be dragged completely out of reach.
    layer.x = layer.x.clamp(-b.w / 2.0 + 0.5, w + b.w / 2.0 - 0.5);
    layer.y = layer.y.clamp(-b.h / 2.0 + 0.5, h + b.h / 2.0 - 0.5);
}

/// Live price for one piece, computed with the same function the server
/// uses. The server always re-prices the order.
fn unit_price(ctx: Ctx) -> Option<Tiyin> {
    ctx.redraw.track(); // text sizes change when fonts arrive
    let garment = ctx.garment()?;
    let (tiers, discounts) = ctx.catalog.with(|c| c.as_ref().map(|c| (c.price_tiers.clone(), c.quantity_discounts.clone())))?;
    ctx.draft.with(|d| {
        let doc = d.doc.as_ref()?;
        let areas = Side::ALL
            .iter()
            .map(|s| garment.area(*s).map(|a| display::printed_area_cm2(doc.layers(*s), a.width_cm, a.height_cm, &Fonts).ceil() as u32).unwrap_or(0))
            .collect();
        let input = QuoteInput { garment_base: garment.base_price, side_areas_cm2: areas, quantity: 1, delivery_fee: Tiyin::ZERO };
        match pricing::quote(&input, &PriceTable { tiers, discounts }) {
            Ok(q) => Some(q.unit_price),
            // Nothing printed yet: show the bare garment price.
            Err(_) => Some(garment.base_price),
        }
    })
}

/// Bring the stored draft in line with the garment that was opened: keep
/// the artwork when switching garments, start fresh when there is none.
fn prepare_draft(ctx: Ctx, garment: &GarmentDto) {
    let current = ctx.draft.with_untracked(|d| d.doc.clone());
    let doc = match current {
        Some(doc) if doc.garment == garment.slug => doc,
        Some(mut doc) => {
            doc.garment = garment.slug.clone();
            if garment.color(&doc.color).is_none() {
                doc.color = garment.colors.first().map(|c| c.slug.clone()).unwrap_or_default();
            }
            // A different garment is a different design on the server.
            ctx.draft.update(|d| {
                d.slug = None;
                d.edit_token = None;
                d.saved_doc = None;
            });
            doc
        }
        None => state::new_doc_for(garment),
    };
    ensure_doc_fonts(&doc);
    ctx.set_doc(doc);
}

#[component]
pub fn EditorPage(garment: Option<String>, shared: Option<String>) -> impl IntoView {
    let ctx = ctx();
    let router = use_router();
    let ed = Ed {
        ctx,
        selected: RwSignal::new(None),
        tool: RwSignal::new(Tool::None),
        history: StoredValue::new(History::default()),
        hist_state: RwSignal::new((false, false)),
        drag: StoredValue::new(None),
        live_stroke: RwSignal::new(None),
        draw_layer: StoredValue::new(None),
        brush_color: RwSignal::new(Color::rgb(255, 223, 0)),
        brush_cm: RwSignal::new(0.5),
        busy: RwSignal::new(false),
    };
    let is_shared = shared.is_some();
    let shared_slug = StoredValue::new(shared.clone());
    let ready = RwSignal::new(false);
    ensure_font(FontId::Montserrat);

    // Set up the document once the catalog is known.
    if let Some(slug) = shared.clone() {
        leptos::task::spawn_local(async move {
            match api::get::<DesignDto>(&format!("/api/designs/{slug}")).await {
                Ok(dto) => {
                    ensure_doc_fonts(&dto.doc);
                    // Opened as a copy: no slug or token, so edits never
                    // touch the original.
                    ctx.draft.set(state::Draft { doc: Some(dto.doc), ..Default::default() });
                    ctx.persist_draft();
                    ed.history.update_value(|h| h.last = ed.doc());
                    ready.set(true);
                }
                Err(e) => {
                    ctx.show_error(&e);
                    router.go(Route::Home);
                }
            }
        });
    } else if let Some(slug) = garment.clone() {
        Effect::new(move |done: Option<bool>| {
            if done == Some(true) {
                return true;
            }
            let found = ctx.catalog.with(|c| c.as_ref().map(|c| c.garments.iter().find(|g| g.slug == slug).cloned()));
            match found {
                Some(Some(g)) => {
                    prepare_draft(ctx, &g);
                    ed.history.update_value(|h| h.last = ed.doc());
                    ready.set(true);
                    true
                }
                Some(None) => {
                    router.go(Route::Home);
                    true
                }
                None => false, // catalog still loading
            }
        });
    }

    let canvas_ref = NodeRef::<leptos::html::Canvas>::new();
    let file_ref = NodeRef::<leptos::html::Input>::new();

    // Current CSS width of the canvas, re-read on every paint.
    let css_width = move || canvas_ref.get_untracked().and_then(|c: HtmlCanvasElement| c.parent_element()).map(|p| p.client_width() as f32).unwrap_or(0.0);

    // Repaint when anything visible changes.
    Effect::new(move |_| {
        ctx.redraw.track();
        let selected = ed.selected.get();
        let side = ctx.side.get();
        let live = ed.live_stroke.get();
        let Some(canvas) = canvas_ref.get() else { return };
        let (Some(garment), Some(area)) = (ctx.garment(), ed.area()) else { return };
        ctx.draft.with(|d| {
            let Some(doc) = d.doc.as_ref() else { return };
            let css_w = css_width();
            if css_w < 10.0 {
                return;
            }
            let view = View::around(&area);
            let scale = draw::fit_canvas(&canvas, css_w, css_w * view.aspect());
            let Some(c) = draw::context_2d(&canvas) else { return };
            let color = garment.color(&doc.color).and_then(|c| Color::parse(&c.hex)).unwrap_or(Color::BLACK);

            let mut layers = doc.layers(side).clone();
            if let Some(stroke) = live.clone() {
                layers.push(Layer::new("live", 0.0, 0.0, LayerKind::Draw(DrawLayer { strokes: vec![stroke] })));
            }
            draw::draw_mockup(
                &c,
                css_w,
                scale,
                &view,
                garment.kind,
                side,
                color,
                Some(&area),
                &layers,
                &MockupOpts { show_area: true, background: Some("#2b2d34") },
            );

            // Selection frame and handles, drawn in CSS-pixel space.
            if let Some(layer) = selected.as_ref().and_then(|id| doc.layers(side).iter().find(|l| l.id == *id)) {
                let base = draw::cm_to_canvas(&area, &view, css_w);
                let h = handles(layer, &base);
                let _ = c.set_transform(scale as f64, 0.0, 0.0, scale as f64, 0.0, 0.0);
                c.set_stroke_style_str("#ffdf00");
                c.set_line_width(2.0);
                c.begin_path();
                c.move_to(h.corners[0].0 as f64, h.corners[0].1 as f64);
                for p in &h.corners[1..] {
                    c.line_to(p.0 as f64, p.1 as f64);
                }
                c.close_path();
                c.stroke();
                let top_mid = ((h.corners[0].0 + h.corners[1].0) / 2.0, (h.corners[0].1 + h.corners[1].1) / 2.0);
                c.begin_path();
                c.move_to(top_mid.0 as f64, top_mid.1 as f64);
                c.line_to(h.rotate.0 as f64, h.rotate.1 as f64);
                c.stroke();
                for (p, fill) in [(h.scale, "#ffdf00"), (h.rotate, "#ffffff")] {
                    c.begin_path();
                    let _ = c.arc(p.0 as f64, p.1 as f64, 11.0, 0.0, std::f64::consts::TAU);
                    c.set_fill_style_str(fill);
                    c.fill();
                    c.set_stroke_style_str("#0d0e11");
                    c.stroke();
                }
                let _ = c.reset_transform();
            }
        });
    });

    // Repaint on resize/rotation.
    let on_resize = window_event_listener(leptos::ev::resize, move |_| ctx.redraw.update(|n| *n += 1));
    on_cleanup(move || on_resize.remove());

    // ---------------------------------------------------- pointer input

    let pointer_cm = move |ev: &PointerEvent| -> Option<((f32, f32), (f32, f32), Affine)> {
        let area = ed.area_untracked()?;
        let base = draw::cm_to_canvas(&area, &View::around(&area), css_width());
        let px = (ev.offset_x() as f32, ev.offset_y() as f32);
        let cm = base.invert()?.apply(px.0, px.1);
        Some((px, cm, base))
    };

    let on_down = move |ev: PointerEvent| {
        let Some((px, cm, base)) = pointer_cm(&ev) else { return };
        // Keep receiving move/up events even if the finger leaves the canvas.
        if let Some(target) = ev.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
            let _ = target.set_pointer_capture(ev.pointer_id());
        }
        ev.prevent_default();

        if ed.tool.get_untracked() == Tool::Draw {
            ed.live_stroke.set(Some(Stroke { points: vec![[cm.0, cm.1]], width_cm: ed.brush_cm.get_untracked(), color: ed.brush_color.get_untracked() }));
            return;
        }
        let side = ctx.side.get_untracked();
        let Some(doc) = ed.doc() else { return };
        let layers = doc.layers(side);

        // Handles of the selected layer take priority over layers under them.
        if let Some(layer) = ed.selected.get_untracked().and_then(|id| layers.iter().find(|l| l.id == id)) {
            let h = handles(layer, &base);
            let kind = if near(px, h.scale, HANDLE_RADIUS) {
                Some(DragKind::Scale)
            } else if near(px, h.rotate, HANDLE_RADIUS) {
                Some(DragKind::Rotate)
            } else {
                None
            };
            if let Some(kind) = kind {
                ed.drag.set_value(Some(Drag { kind, start: cm, orig: layer.clone() }));
                return;
            }
        }
        match display::hit_test(layers, cm.0, cm.1, 0.5, &Fonts) {
            Some(layer) => {
                ed.selected.set(Some(layer.id.clone()));
                ed.drag.set_value(Some(Drag { kind: DragKind::Move, start: cm, orig: layer.clone() }));
            }
            None => ed.selected.set(None),
        }
    };

    let on_move = move |ev: PointerEvent| {
        let Some((_, cm, _)) = pointer_cm(&ev) else { return };
        if ed.live_stroke.with_untracked(|s| s.is_some()) {
            ed.live_stroke.update(|s| {
                if let Some(s) = s {
                    // Skip points closer than 1 mm: keeps strokes small.
                    let far = s.points.last().is_none_or(|p| (p[0] - cm.0).powi(2) + (p[1] - cm.1).powi(2) > 0.01);
                    if far && s.points.len() < 2000 {
                        s.points.push([cm.0, cm.1]);
                    }
                }
            });
            return;
        }
        let Some(drag) = ed.drag.get_value() else { return };
        let Some(area) = ed.area_untracked() else { return };
        let o = &drag.orig;
        let mut next = o.clone();
        match drag.kind {
            DragKind::Move => {
                next.x = o.x + cm.0 - drag.start.0;
                next.y = o.y + cm.1 - drag.start.1;
                snap_move(&mut next, &area);
            }
            DragKind::Scale => {
                let d0 = ((drag.start.0 - o.x).powi(2) + (drag.start.1 - o.y).powi(2)).sqrt().max(0.05);
                let d1 = ((cm.0 - o.x).powi(2) + (cm.1 - o.y).powi(2)).sqrt();
                next.scale = (o.scale * d1 / d0).clamp(0.05, 30.0);
                // Do not let a layer shrink below about 5 mm.
                let b = display::local_bounds(&next, &Fonts);
                let min = 0.5 / b.w.max(b.h).max(0.01);
                next.scale = next.scale.max(min);
            }
            DragKind::Rotate => {
                let a0 = (drag.start.1 - o.y).atan2(drag.start.0 - o.x);
                let a1 = (cm.1 - o.y).atan2(cm.0 - o.x);
                let mut deg = (o.rotation + (a1 - a0).to_degrees()).rem_euclid(360.0);
                // Snap to 45° steps when within 4°.
                let nearest = (deg / 45.0).round() * 45.0;
                if (deg - nearest).abs() < 4.0 {
                    deg = nearest.rem_euclid(360.0);
                }
                next.rotation = deg;
            }
        }
        ed.mutate_selected(|l| *l = next);
    };

    let on_up = move |_ev: PointerEvent| {
        if let Some(stroke) = ed.live_stroke.get_untracked() {
            ed.live_stroke.set(None);
            let side = ctx.side.get_untracked();
            // All strokes of one drawing session share a layer.
            let existing = ed.draw_layer.get_value();
            let id = existing.clone().unwrap_or_else(new_layer_id);
            ed.mutate(|doc| {
                let layers = doc.layers_mut(side);
                match layers.iter_mut().find(|l| Some(&l.id) == existing.as_ref()) {
                    Some(Layer { kind: LayerKind::Draw(d), .. }) => d.strokes.push(stroke),
                    _ => layers.push(Layer::new(id.clone(), 0.0, 0.0, LayerKind::Draw(DrawLayer { strokes: vec![stroke] }))),
                }
            });
            ed.draw_layer.set_value(Some(id));
            ed.commit();
            return;
        }
        if ed.drag.get_value().is_some() {
            ed.drag.set_value(None);
            ed.commit();
        }
    };

    // ------------------------------------------------------- image upload

    let on_file = move |_| {
        let Some(input) = file_ref.get_untracked() else { return };
        let Some(file) = input.files().and_then(|f| f.get(0)) else { return };
        input.set_value(""); // allow picking the same file again
        let max = ctx.catalog.with_untracked(|c| c.as_ref().map(|c| c.max_upload_bytes).unwrap_or(10 * 1024 * 1024));
        if file.size() > max as f64 {
            ctx.show_toast(t("error.upload_too_large"), true);
            return;
        }
        ed.busy.set(true);
        leptos::task::spawn_local(async move {
            let result = match api::read_file(&file).await {
                Some(bytes) => api::post_bytes::<AssetDto>("/api/assets", &bytes, &[]).await,
                None => Err(api::ApiFail { status: 0, code: "upload_invalid".into() }),
            };
            ed.busy.set(false);
            match result {
                Ok(asset) => {
                    let Some(area) = ed.area_untracked() else { return };
                    // Fit inside 70 % of the area, keeping the aspect ratio.
                    let aspect = asset.height as f32 / asset.width.max(1) as f32;
                    let mut w = area.width_cm * 0.7;
                    let mut h = w * aspect;
                    if h > area.height_cm * 0.7 {
                        h = area.height_cm * 0.7;
                        w = h / aspect;
                    }
                    let kind = LayerKind::Image(ImageLayer { asset: asset.id, width_cm: w, height_cm: h, px_width: asset.width, px_height: asset.height });
                    ed.add_layer(kind, area.width_cm / 2.0, area.height_cm / 2.0);
                }
                Err(e) => ctx.show_error(&e),
            }
        });
    };

    // ------------------------------------------------------------ actions

    let add_text = move |_| {
        ed.set_tool(Tool::None);
        let Some(area) = ed.area_untracked() else { return };
        let garment_color =
            ctx.garment().and_then(|g| ctx.draft.with_untracked(|d| d.doc.as_ref().and_then(|doc| g.color(&doc.color).and_then(|c| Color::parse(&c.hex)))));
        let kind = LayerKind::Text(TextLayer {
            text: t("editor.text_default").to_string(),
            font: FontId::Montserrat,
            size_cm: area.width_cm * 0.13,
            color: contrast_ink(garment_color.unwrap_or(Color::BLACK)),
            outline: None,
            curve: 0.0,
            align: Align::Center,
        });
        ed.add_layer(kind, area.width_cm / 2.0, area.height_cm * 0.3);
    };

    let share = move |_| {
        if ed.busy.get_untracked() {
            return;
        }
        ed.set_tool(Tool::None);
        ed.busy.set(true);
        leptos::task::spawn_local(async move {
            let saved = ctx.ensure_saved().await;
            ed.busy.set(false);
            match saved {
                Ok(slug) => {
                    let origin = web_sys::window().and_then(|w| w.location().origin().ok()).unwrap_or_default();
                    // In the mobile apps the page origin is the device; share
                    // the public site address instead.
                    let base = if platform::is_native() { platform::api_base() } else { origin };
                    let url = format!("{base}/d/{slug}");
                    let brand = ctx.catalog.with_untracked(|c| c.as_ref().map(|c| c.brand.clone()).unwrap_or_default());
                    match platform::await_string(platform::share(&brand, Some(&url), None, "")).await.as_str() {
                        "copied" => ctx.show_toast(t("common.copied"), false),
                        "failed" => ctx.show_toast(url, false),
                        _ => {}
                    }
                }
                Err(e) => ctx.show_error(&e),
            }
        });
    };

    let go_tryon = move |_| {
        ed.set_tool(Tool::None);
        if ctx.draft.with_untracked(|d| d.doc.as_ref().is_none_or(|doc| doc.is_empty())) {
            ctx.show_toast(t("editor.empty_hint"), true);
            return;
        }
        router.go(Route::TryOn);
    };

    let title = Signal::derive(move || ctx.garment().map(|g| g.name.get(ctx.lang.get()).to_string()).unwrap_or_default());
    let is_empty = move || ctx.draft.with(|d| d.doc.as_ref().is_none_or(|doc| doc.layers(ctx.side.get()).is_empty()));
    let selected_layer = Memo::new(move |_| {
        let id = ed.selected.get()?;
        let side = ctx.side.get();
        ctx.draft.with(|d| d.doc.as_ref()?.layers(side).iter().find(|l| l.id == id).cloned())
    });

    view! {
        <TopBar title=title>
            <button class="icon-btn" aria-label=move || t("editor.undo") disabled=move || !ed.hist_state.get().0 on:click=move |_| ed.undo()>{icons::undo()}</button>
            <button class="icon-btn" aria-label=move || t("editor.redo") disabled=move || !ed.hist_state.get().1 on:click=move |_| ed.redo()>{icons::redo()}</button>
            <button class="icon-btn" aria-label=move || t("editor.share_link") on:click=share>{icons::share()}</button>
        </TopBar>
        <main class="page has-bar">
            <Show when=move || is_shared>
                <div class="note">{move || t("editor.shared_view")}</div>
                <button class="link-btn" on:click=move |_| {
                    // Hand the design over to the report form.
                    state::store_set("qd.report", &shared_slug.get_value().unwrap_or_default());
                    router.go(Route::Legal);
                }>{move || t("editor.report")}</button>
            </Show>
            <Show when=move || ready.get() fallback=move || view! { <p class="muted">{move || t("common.loading")}</p> }>
                <div class="stage">
                    <canvas node_ref=canvas_ref on:pointerdown=on_down on:pointermove=on_move on:pointerup=on_up on:pointercancel=on_up></canvas>
                    <Show when=move || is_empty() && ed.tool.get() != Tool::Draw>
                        <div class="hint">{move || t("editor.empty_hint")}</div>
                    </Show>
                    <div class="side-toggle">
                        {Side::ALL
                            .into_iter()
                            .map(|s| {
                                let label = if s == Side::Front { "common.front" } else { "common.back_side" };
                                view! {
                                    <button
                                        class:on=move || ctx.side.get() == s
                                        on:click=move |_| {
                                            ed.set_tool(Tool::None);
                                            ed.selected.set(None);
                                            ctx.side.set(s);
                                        }
                                    >
                                        {move || t(label)}
                                    </button>
                                }
                            })
                            .collect_view()}
                    </div>
                </div>
                <p class="muted small center">
                    {move || ed.area().map(|a| tf("editor.print_area", &[("w", &format!("{:.0}", a.width_cm)), ("h", &format!("{:.0}", a.height_cm))]))}
                </p>

                <input node_ref=file_ref type="file" accept="image/png,image/jpeg,image/webp" hidden on:change=on_file/>
                <div class="tools">
                    <button class="tool" on:click=add_text>{icons::text()}<span>{move || t("editor.text")}</span></button>
                    <button
                        class="tool"
                        disabled=move || ed.busy.get()
                        on:click=move |_| {
                            ed.set_tool(Tool::None);
                            if let Some(i) = file_ref.get_untracked() {
                                i.click();
                            }
                        }
                    >
                        {icons::image()}
                        <span>{move || if ed.busy.get() { t("editor.uploading") } else { t("editor.image") }}</span>
                    </button>
                    <ToolButton ed=ed tool=Tool::Draw label="editor.draw" icon=icons::brush().into_any()/>
                    <ToolButton ed=ed tool=Tool::Shapes label="editor.shapes" icon=icons::shapes().into_any()/>
                    <ToolButton ed=ed tool=Tool::Stickers label="editor.stickers" icon=icons::sticker().into_any()/>
                    <ToolButton ed=ed tool=Tool::Templates label="editor.templates" icon=icons::template().into_any()/>
                </div>

                {move || match ed.tool.get() {
                    Tool::Draw => view! { <DrawPanel ed=ed/> }.into_any(),
                    Tool::Shapes => view! { <ShapePicker ed=ed/> }.into_any(),
                    Tool::Stickers => view! { <StickerPicker ed=ed/> }.into_any(),
                    Tool::Templates => view! { <TemplatePicker ed=ed/> }.into_any(),
                    Tool::None => ().into_any(),
                }}

                // The panel is rebuilt only when a different layer is
                // selected (not on every keystroke), so inputs keep focus.
                {move || {
                    let id = ed.selected.get();
                    let has = id.is_some() && selected_layer.with_untracked(|l| l.is_some());
                    has.then(|| view! { <LayerPanel ed=ed layer=selected_layer/> })
                }}

                <GarmentOptions ed=ed/>
            </Show>
        </main>
        <div class="sticky-bar">
            <div class="grow">
                <div class="price num">{move || unit_price(ctx).map(|p| p.format_sum()).unwrap_or_default()}<small>{move || t("common.sum")}</small></div>
            </div>
            <button class="btn btn-primary" on:click=go_tryon>{icons::camera()}{move || t("editor.try_on")}</button>
        </div>
    }
}

#[component]
fn ToolButton(ed: Ed, tool: Tool, label: &'static str, icon: AnyView) -> impl IntoView {
    view! {
        <button class="tool" class:on=move || ed.tool.get() == tool on:click=move |_| ed.set_tool(if ed.tool.get_untracked() == tool { Tool::None } else { tool })>
            {icon}
            <span>{move || t(label)}</span>
        </button>
    }
}

/// A row of colour swatches plus a free colour input. `none` adds a
/// "no colour" choice (for outlines and fills).
#[component]
fn ColorRow(#[prop(into)] value: Signal<Option<Color>>, on_pick: Callback<Option<Color>>, #[prop(optional)] none: bool) -> impl IntoView {
    view! {
        <div class="swatches">
            <Show when=move || none>
                <button class="swatch none" class:on=move || value.get().is_none() aria-label=move || t("common.none") on:click=move |_| on_pick.run(None)></button>
            </Show>
            {PALETTE
                .into_iter()
                .map(|hex| {
                    let color = Color::parse(hex).unwrap_or(Color::WHITE);
                    view! { <button class="swatch" style=format!("background:{hex}") class:on=move || value.get() == Some(color) aria-label=hex on:click=move |_| on_pick.run(Some(color))></button> }
                })
                .collect_view()}
            <input
                type="color"
                class="swatch"
                style="padding:0"
                prop:value=move || value.get().map(|c| Color([c.0[0], c.0[1], c.0[2], 255]).to_hex()).unwrap_or_else(|| "#ffdf00".into())
                on:change=move |ev| on_pick.run(Color::parse(&event_target_value(&ev)))
            />
        </div>
    }
}

#[component]
fn DrawPanel(ed: Ed) -> impl IntoView {
    view! {
        <div class="card">
            <ColorRow value=Signal::derive(move || Some(ed.brush_color.get())) on_pick=Callback::new(move |c: Option<Color>| ed.brush_color.set(c.unwrap_or(Color::WHITE)))/>
            <label class="field">
                <span>{move || t("editor.brush")}</span>
                <input
                    class="range"
                    type="range"
                    min="0.1"
                    max="2"
                    step="0.1"
                    prop:value=move || ed.brush_cm.get().to_string()
                    on:input=move |ev| ed.brush_cm.set(event_target_value(&ev).parse().unwrap_or(0.5))
                />
            </label>
            <button class="btn btn-ghost btn-sm" on:click=move |_| ed.set_tool(Tool::None)>{icons::check()}{move || t("common.done")}</button>
        </div>
    }
}

fn default_ink(ed: Ed) -> Color {
    let hex = ed.ctx.garment().and_then(|g| ed.ctx.draft.with_untracked(|d| d.doc.as_ref().and_then(|doc| g.color(&doc.color).map(|c| c.hex.clone()))));
    contrast_ink(hex.and_then(|h| Color::parse(&h)).unwrap_or(Color::BLACK))
}

#[component]
fn ShapePicker(ed: Ed) -> impl IntoView {
    let icon = |kind: ShapeKind| {
        let d = design::svgpath::to_svg(&design::shapes::shape_path(kind, 80.0, 80.0));
        format!(r#"<path transform="translate(50 50)" d="{d}"/>"#)
    };
    view! {
        <div class="picker">
            {ShapeKind::ALL
                .into_iter()
                .map(|kind| {
                    view! {
                        <button on:click=move |_| {
                            let Some(a) = ed.area_untracked() else { return };
                            let s = a.width_cm * 0.3;
                            let shape = ShapeLayer { shape: kind, width_cm: s, height_cm: s, fill: Some(default_ink(ed)), stroke: None };
                            ed.add_layer(LayerKind::Shape(shape), a.width_cm / 2.0, a.height_cm / 2.0);
                            ed.set_tool(Tool::None);
                        }>
                            <svg viewBox="0 0 100 100" inner_html=icon(kind)></svg>
                        </button>
                    }
                })
                .collect_view()}
        </div>
    }
}

#[component]
fn StickerPicker(ed: Ed) -> impl IntoView {
    view! {
        <div class="picker">
            {design::stickers::STICKERS
                .iter()
                .map(|s| {
                    let id = s.id;
                    let inner = format!(r#"<path fill-rule="evenodd" d="{}"/>"#, s.d);
                    view! {
                        <button on:click=move |_| {
                            let Some(a) = ed.area_untracked() else { return };
                            let sticker = StickerLayer { sticker: id.to_string(), width_cm: a.width_cm * 0.3, color: default_ink(ed) };
                            ed.add_layer(LayerKind::Sticker(sticker), a.width_cm / 2.0, a.height_cm / 2.0);
                            ed.set_tool(Tool::None);
                        }>
                            <svg viewBox="0 0 100 100" inner_html=inner></svg>
                        </button>
                    }
                })
                .collect_view()}
        </div>
    }
}

/// Thumbnail of a template as inline SVG, built from the same display list
/// the canvas uses.
fn template_svg(layers: &[Layer]) -> String {
    let mut out = String::new();
    for op in layers.iter().flat_map(|l| display::layer_ops(l, &Fonts)) {
        if let DrawOp::Path { path, paint, transform: m } = op {
            let d = design::svgpath::to_svg(&path);
            let matrix = format!("matrix({} {} {} {} {} {})", m.a, m.b, m.c, m.d, m.e, m.f);
            match paint {
                Paint::Fill(..) => out.push_str(&format!(r#"<path d="{d}" transform="{matrix}" fill-rule="evenodd"/>"#)),
                Paint::Stroke { width, .. } => {
                    out.push_str(&format!(r#"<path d="{d}" transform="{matrix}" style="fill:none;stroke:#fff;stroke-width:{width}"/>"#))
                }
            }
        }
    }
    out
}

#[component]
fn TemplatePicker(ed: Ed) -> impl IntoView {
    // Templates use several fonts; fetch them when the picker opens.
    for f in [FontId::Oswald, FontId::RubikMono, FontId::Pacifico, FontId::Caveat] {
        ensure_font(f);
    }
    view! {
        <div class="picker wide">
            {design::templates::TEMPLATES
                .iter()
                .map(|tpl| {
                    let thumb = move || {
                        ed.ctx.redraw.track();
                        let (w, h) = ed.area().map(|a| (a.width_cm, a.height_cm)).unwrap_or((30.0, 40.0));
                        // Templates occupy the top part of the area.
                        (format!("0 0 {w} {}", h * 0.62), template_svg(&tpl.layers(w, h, Color::WHITE)))
                    };
                    view! {
                        <button on:click=move |_| {
                            let Some(a) = ed.area_untracked() else { return };
                            let side = ed.ctx.side.get_untracked();
                            let mut layers = tpl.layers(a.width_cm, a.height_cm, default_ink(ed));
                            for l in &mut layers {
                                l.id = new_layer_id();
                            }
                            ed.mutate(|doc| *doc.layers_mut(side) = layers);
                            ed.selected.set(None);
                            ed.commit();
                            ed.set_tool(Tool::None);
                            platform::haptic("light");
                        }>
                            {move || {
                                let (vb, inner) = thumb();
                                view! { <svg viewBox=vb style="width:86%;height:86%" inner_html=inner></svg> }
                            }}
                        </button>
                    }
                })
                .collect_view()}
        </div>
    }
}

/// Controls for the selected layer.
#[component]
fn LayerPanel(ed: Ed, layer: Memo<Option<Layer>>) -> impl IntoView {
    let ctx = ed.ctx;
    let kind_tag = layer.with_untracked(|l| match l.as_ref().map(|l| &l.kind) {
        Some(LayerKind::Text(_)) => 1,
        Some(LayerKind::Image(_)) => 2,
        Some(LayerKind::Shape(_)) => 3,
        Some(LayerKind::Sticker(_)) => 4,
        _ => 0,
    });
    let text = move || {
        layer.with(|l| match l.as_ref().map(|l| &l.kind) {
            Some(LayerKind::Text(t)) => Some(t.clone()),
            _ => None,
        })
    };
    // Apply a change to the selected layer and record it for undo.
    let apply = move |f: Box<dyn FnOnce(&mut Layer)>| {
        ed.mutate_selected(f);
        ed.commit();
    };
    let edit_text = move |f: Box<dyn FnOnce(&mut TextLayer)>, commit: bool| {
        ed.mutate_selected(|l| {
            if let LayerKind::Text(t) = &mut l.kind {
                f(t);
            }
        });
        if commit {
            ed.commit();
        }
    };

    let reorder = move |forward: bool| {
        let Some(id) = ed.selected.get_untracked() else { return };
        let side = ctx.side.get_untracked();
        ed.mutate(|doc| {
            let layers = doc.layers_mut(side);
            if let Some(i) = layers.iter().position(|l| l.id == id) {
                let j = if forward { (i + 1).min(layers.len() - 1) } else { i.saturating_sub(1) };
                layers.swap(i, j);
            }
        });
        ed.commit();
    };
    let duplicate = move |_| {
        let Some(mut copy) = layer.get_untracked() else { return };
        copy.id = new_layer_id();
        copy.x += 1.0;
        copy.y += 1.0;
        let id = copy.id.clone();
        let side = ctx.side.get_untracked();
        ed.mutate(|doc| doc.layers_mut(side).push(copy));
        ed.selected.set(Some(id));
        ed.commit();
    };
    let remove = move |_| {
        let Some(id) = ed.selected.get_untracked() else { return };
        let side = ctx.side.get_untracked();
        ed.mutate(|doc| doc.layers_mut(side).retain(|l| l.id != id));
        ed.selected.set(None);
        ed.commit();
        platform::haptic("light");
    };
    let center = move |_| {
        let Some(a) = ed.area_untracked() else { return };
        apply(Box::new(move |l| {
            let b = display::layer_bbox(l, &Fonts);
            l.x += a.width_cm / 2.0 - b.center().0;
        }));
    };
    let remove_bg = move |_| {
        let Some(asset) = layer.with_untracked(|l| match l.as_ref().map(|l| &l.kind) {
            Some(LayerKind::Image(i)) => Some(i.asset.clone()),
            _ => None,
        }) else {
            return;
        };
        ed.busy.set(true);
        leptos::task::spawn_local(async move {
            let r = api::post::<_, AssetDto>(&format!("/api/assets/{asset}/remove-bg"), &serde_json::json!({})).await;
            ed.busy.set(false);
            match r {
                Ok(new) => {
                    ed.mutate_selected(|l| {
                        if let LayerKind::Image(i) = &mut l.kind {
                            i.asset = new.id;
                        }
                    });
                    ed.commit();
                }
                Err(e) => ctx.show_error(&e),
            }
        });
    };

    let dpi_note = move || {
        layer.with(|l| match l.as_ref().map(|l| (&l.kind, l.scale)) {
            Some((LayerKind::Image(i), scale)) => match dpi_quality(i.dpi(scale)) {
                DpiQuality::Good => None,
                DpiQuality::Low => Some(("note warn", "editor.dpi_low")),
                DpiQuality::Bad => Some(("note bad", "editor.dpi_bad")),
            },
            _ => None,
        })
    };

    view! {
        <div class="card">
            {(kind_tag == 1)
                .then(|| {
                    view! {
                        <textarea
                            class="textarea"
                            rows="2"
                            maxlength="200"
                            placeholder=move || t("editor.text_placeholder")
                            prop:value=move || text().map(|t| t.text).unwrap_or_default()
                            on:input=move |ev| {
                                let v = event_target_value(&ev);
                                edit_text(Box::new(move |t| t.text = v), false);
                            }
                            on:change=move |_| ed.commit()
                        ></textarea>
                        <div class="chips">
                            {FontId::ALL
                                .into_iter()
                                .map(|f| {
                                    view! {
                                        <button
                                            class="chip"
                                            class:on=move || text().is_some_and(|t| t.font == f)
                                            on:click=move |_| {
                                                ensure_font(f);
                                                edit_text(Box::new(move |t| t.font = f), true);
                                            }
                                        >
                                            {f.label()}
                                        </button>
                                    }
                                })
                                .collect_view()}
                        </div>
                        <ColorRow value=Signal::derive(move || text().map(|t| t.color)) on_pick=Callback::new(move |c: Option<Color>| {
                            if let Some(c) = c {
                                edit_text(Box::new(move |t| t.color = c), true);
                            }
                        })/>
                        <label class="field">
                            <span>{move || t("editor.curve")}</span>
                            <input
                                class="range"
                                type="range"
                                min="-100"
                                max="100"
                                step="5"
                                prop:value=move || text().map(|t| t.curve.to_string()).unwrap_or_default()
                                on:input=move |ev| {
                                    let v: f32 = event_target_value(&ev).parse().unwrap_or(0.0);
                                    // A little dead zone makes "straight" easy to hit.
                                    let v = if v.abs() < 8.0 { 0.0 } else { v };
                                    edit_text(Box::new(move |t| t.curve = v), false);
                                }
                                on:change=move |_| ed.commit()
                            />
                        </label>
                        <div class="field">
                            <span>{move || t("editor.outline")}</span>
                            <ColorRow
                                none=true
                                value=Signal::derive(move || text().and_then(|t| t.outline.map(|o| o.color)))
                                on_pick=Callback::new(move |c: Option<Color>| {
                                    edit_text(Box::new(move |t| t.outline = c.map(|color| Outline { color, width_cm: (t.size_cm * 0.045).max(0.05) })), true);
                                })
                            />
                        </div>
                        <div class="seg">
                            {[(Align::Left, "⟸"), (Align::Center, "⟺"), (Align::Right, "⟹")]
                                .into_iter()
                                .map(|(a, glyph)| {
                                    view! { <button class:on=move || text().is_some_and(|t| t.align == a) on:click=move |_| edit_text(Box::new(move |t| t.align = a), true)>{glyph}</button> }
                                })
                                .collect_view()}
                        </div>
                    }
                })}
            {(kind_tag == 2)
                .then(|| {
                    view! {
                        {move || dpi_note().map(|(class, key)| view! { <div class=class>{move || t(key)}</div> })}
                        <button class="btn btn-ghost btn-sm" disabled=move || ed.busy.get() on:click=remove_bg>{move || t("editor.remove_bg")}</button>
                        <p class="muted small">{move || t("editor.upload_rights")}</p>
                    }
                })}
            {(kind_tag == 3)
                .then(|| {
                    let shape = move || layer.with(|l| match l.as_ref().map(|l| &l.kind) {
                        Some(LayerKind::Shape(s)) => Some(s.clone()),
                        _ => None,
                    });
                    view! {
                        <div class="field">
                            <span>{move || t("editor.fill")}</span>
                            <ColorRow
                                none=true
                                value=Signal::derive(move || shape().and_then(|s| s.fill))
                                on_pick=Callback::new(move |c: Option<Color>| {
                                    apply(Box::new(move |l| {
                                        if let LayerKind::Shape(s) = &mut l.kind {
                                            // A shape needs a fill or an outline to be visible.
                                            if c.is_some() || s.stroke.is_some() {
                                                s.fill = c;
                                            }
                                        }
                                    }));
                                })
                            />
                        </div>
                        <div class="field">
                            <span>{move || t("editor.outline")}</span>
                            <ColorRow
                                none=true
                                value=Signal::derive(move || shape().and_then(|s| s.stroke.map(|o| o.color)))
                                on_pick=Callback::new(move |c: Option<Color>| {
                                    apply(Box::new(move |l| {
                                        if let LayerKind::Shape(s) = &mut l.kind {
                                            if c.is_some() || s.fill.is_some() {
                                                s.stroke = c.map(|color| Outline { color, width_cm: (s.width_cm * 0.05).clamp(0.1, 1.0) });
                                            }
                                        }
                                    }));
                                })
                            />
                        </div>
                    }
                })}
            {(kind_tag == 4)
                .then(|| {
                    let color = move || layer.with(|l| match l.as_ref().map(|l| &l.kind) {
                        Some(LayerKind::Sticker(s)) => Some(s.color),
                        _ => None,
                    });
                    view! {
                        <ColorRow value=Signal::derive(color) on_pick=Callback::new(move |c: Option<Color>| {
                            if let Some(c) = c {
                                apply(Box::new(move |l| {
                                    if let LayerKind::Sticker(s) = &mut l.kind {
                                        s.color = c;
                                    }
                                }));
                            }
                        })/>
                    }
                })}
            <div class="actions">
                <button class="icon-btn" aria-label=move || t("editor.center") on:click=center>{icons::center()}</button>
                <button class="icon-btn" aria-label=move || t("editor.flip_h") on:click=move |_| apply(Box::new(|l| l.flip_x = !l.flip_x))>{icons::flip()}</button>
                <button class="icon-btn" aria-label=move || t("editor.to_front") on:click=move |_| reorder(true)>{icons::up()}</button>
                <button class="icon-btn" aria-label=move || t("editor.to_back") on:click=move |_| reorder(false)>{icons::down()}</button>
                <button class="icon-btn" aria-label=move || t("editor.duplicate") on:click=duplicate>{icons::copy()}</button>
                <span class="grow"></span>
                <button class="icon-btn" style="color:var(--danger)" aria-label=move || t("common.delete") on:click=remove>{icons::trash()}</button>
            </div>
        </div>
    }
}

/// Garment colour and print finish.
#[component]
fn GarmentOptions(ed: Ed) -> impl IntoView {
    let ctx = ed.ctx;
    let current = move || ctx.draft.with(|d| d.doc.as_ref().map(|doc| doc.color.clone()));
    view! {
        <div class="card">
            <div class="field">
                <span>{move || t("common.color")}</span>
                <div class="swatches">
                    {move || {
                        ctx.garment()
                            .map(|g| g.colors)
                            .unwrap_or_default()
                            .into_iter()
                            .map(|c| {
                                let (slug, slug2) = (c.slug.clone(), c.slug.clone());
                                let name = c.name.clone();
                                view! {
                                    <button
                                        class="swatch"
                                        style=format!("background:{}", c.hex)
                                        class:on=move || current().as_deref() == Some(slug.as_str())
                                        aria-label=move || name.get(ctx.lang.get()).to_string()
                                        on:click=move |_| {
                                            let s = slug2.clone();
                                            ed.mutate(|doc| doc.color = s);
                                            ed.commit();
                                        }
                                    ></button>
                                }
                            })
                            .collect_view()
                    }}
                </div>
            </div>
            <div class="field">
                <span>{move || t("editor.finish")}</span>
                <div class="chips wrap">
                    <button class="chip on">{move || t("editor.finish_standard")}</button>
                    // Foil exists in the data model but cannot be ordered yet.
                    <button class="chip" disabled=true>{move || t("editor.finish_foil_gold")}<span class="soon">{move || t("editor.soon")}</span></button>
                    <button class="chip" disabled=true>{move || t("editor.finish_foil_silver")}<span class="soon">{move || t("editor.soon")}</span></button>
                </div>
            </div>
        </div>
    }
}
