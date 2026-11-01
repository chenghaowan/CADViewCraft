//! 自绘标题栏:应用栏空白处拖动窗口、双击最大化,右端最小化/最大化/关闭三个
//! 窗控按钮,窗口边缘 8px(96 DPI)隐形缩放区。照 SolidCraft titlebar.rs 移植
//! (拖动走 OS 的 move loop,Windows 贴边分屏照常工作)。

use egui::{Color32, CursorIcon, Id, LayerId, Order, PointerButton, Pos2, Rect, ResizeDirection, Sense, Stroke, Ui, ViewportCommand, pos2, vec2};

use crate::theme::{Tokens, RADIUS_SM};

/// 一个窗控按钮的宽度(与 Windows 原生一致)。
pub const BUTTON_WIDTH: f32 = 46.0;
/// 三个窗控按钮的总宽。
pub const WIDTH: f32 = 3.0 * BUTTON_WIDTH;
/// Windows 100% 缩放下的原生缩放边框:SM_CXSIZEFRAME + SM_CXPADDEDBORDER,4+4 px。
pub const RESIZE_BORDER_AT_96_DPI: f32 = 8.0;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Caption {
    Minimize,
    Maximize,
    Close,
}

/// 从左到右。
pub const CAPTIONS: [Caption; 3] = [Caption::Minimize, Caption::Maximize, Caption::Close];

fn caption_id(c: Caption) -> Id {
    Id::new(("titlebar-caption", c as u8))
}

fn maximized(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false))
}

pub fn toggle_maximize(ctx: &egui::Context) {
    ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized(ctx)));
}

/// 应用栏背景:空白处按下拖动移动窗口,双击切换最大化。要先于栏内按钮调用,
/// 让后注册的按钮拿到自己的点击。
pub fn drag_area(ui: &Ui, rect: Rect) {
    let ctx = ui.ctx();
    let resp = ui.interact(rect, Id::new("titlebar-drag"), Sense::click_and_drag());
    // egui 会把从纯点击控件上开始的拖动交给下面的 drag 背景,记下按下是否落在空白处。
    if ctx.input(|i| i.pointer.any_pressed()) {
        let bare = ctx.viewport(|v| v.hits.click.is_some_and(|w| w.id == resp.id));
        ctx.data_mut(|d| d.insert_temp(resp.id, bare));
    }
    if resp.double_clicked() {
        toggle_maximize(ctx);
    } else if resp.drag_started_by(PointerButton::Primary) && ctx.data(|d| d.get_temp::<bool>(resp.id).unwrap_or(false)) {
        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
    }
}

/// 应用栏矩形右端的最小化/最大化/关闭:整栏高、贴窗口右缘。
pub fn caption_buttons(ui: &Ui, bar: Rect) {
    let t = Tokens::get();
    let max = maximized(ui.ctx());
    for (i, c) in CAPTIONS.into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(bar.right() - WIDTH + i as f32 * BUTTON_WIDTH, bar.top()), vec2(BUTTON_WIDTH, bar.height()));
        let resp = ui.interact(r, caption_id(c), Sense::click());
        let fill = match (resp.is_pointer_button_down_on(), resp.hovered()) {
            (true, _) => t.app_bar_hover_subtle,
            (_, true) => t.app_bar_hover_button,
            _ => Color32::TRANSPARENT,
        };
        ui.painter().rect_filled(r, 0.0, fill);
        paint_caption_glyph(ui, c, max, r.center(), Stroke::new(1.0, t.app_bar_text));
        let tip = match c {
            Caption::Minimize => "最小化",
            Caption::Maximize if max => "还原",
            Caption::Maximize => "最大化",
            Caption::Close => "关闭",
        };
        if resp.on_hover_text(tip).clicked() {
            match c {
                Caption::Minimize => ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true)),
                Caption::Maximize => toggle_maximize(ui.ctx()),
                Caption::Close => ui.ctx().send_viewport_cmd(ViewportCommand::Close),
            }
        }
    }
}

/// 最小化横线 / 最大化方框 / 还原双框 / 关闭斜叉(照 SolidCraft glyphs::paint_glyph)。
fn paint_caption_glyph(ui: &Ui, c: Caption, maximized: bool, at: Pos2, s: Stroke) {
    let p = ui.painter();
    let g = Rect::from_center_size(at, vec2(10.0, 10.0));
    match c {
        Caption::Minimize => {
            p.line_segment([g.left_center(), g.right_center()], s);
        }
        Caption::Maximize if maximized => {
            let front = Rect::from_min_max(g.min + vec2(0.0, 2.0), g.max - vec2(2.0, 0.0));
            p.rect_stroke(front, 0.0, s, egui::StrokeKind::Middle);
            p.line(
                vec![
                    g.min + vec2(2.0, 2.0),
                    g.min + vec2(2.0, 0.0),
                    g.right_top(),
                    g.max - vec2(0.0, 2.0),
                    g.max - vec2(2.0, 2.0),
                ],
                s,
            );
        }
        Caption::Maximize => {
            p.rect_stroke(g, 0.0, s, egui::StrokeKind::Middle);
        }
        Caption::Close => {
            p.line_segment([g.left_top(), g.right_bottom()], s);
            p.line_segment([g.right_top(), g.left_bottom()], s);
        }
    }
}

/// 品牌 logo:三个亮中暗面的凸多边形拼成(坐标取自 logo SVG 的 viewBox 357×412)。
/// 照 SolidCraft glyphs::brand_mark_styled。
pub fn brand_mark(p: &egui::Painter, c: Pos2, height: f32, colours: [Color32; 3], bar: Color32) {
    let s = height / 412.217;
    let m = |pts: &[(f32, f32)]| -> Vec<Pos2> { pts.iter().map(|&(x, y)| pos2(c.x + (x - 178.5) * s, c.y + (y - 206.1) * s)).collect() };
    // egui 只填凸形,每个面拆成几块凸片。
    let face = |col: Color32, pts: &[(f32, f32)]| p.add(egui::Shape::convex_polygon(m(pts), col, Stroke::NONE));
    let [light, mid, dark] = colours;
    face(light, &[(3.5, 105.9), (177.0, 5.8), (263.4, 55.6), (263.8, 156.0), (177.0, 406.4)]);
    face(light, &[(177.0, 206.1), (263.8, 156.0), (350.5, 206.2), (350.5, 306.3), (177.0, 406.4)]);
    face(mid, &[(3.5, 105.9), (177.0, 206.1), (177.0, 406.4)]);
    face(mid, &[(3.5, 105.9), (3.5, 205.7), (90.3, 256.2), (177.0, 206.1)]);
    face(mid, &[(177.0, 206.1), (90.3, 256.2), (90.0, 356.2), (177.0, 406.4)]);
    face(dark, &[(264.1, 155.8), (177.0, 206.1), (177.0, 406.4), (263.8, 256.2)]);
    face(dark, &[(263.8, 256.2), (350.5, 206.2), (350.5, 306.3), (177.0, 406.4)]);
    face(dark, &[(177.0, 206.1), (177.0, 406.4), (350.5, 306.3)]);
    face(dark, &[(263.8, 55.9), (263.8, 156.0), (177.1, 105.9)]);
    // 面缝与外轮廓用应用栏底色描出,读作镂空。
    let edge = Stroke::new(10.0 * s, bar);
    let line = |pts: &[(f32, f32)]| p.add(egui::Shape::line(m(pts), edge));
    line(&[(263.8, 55.9), (177.1, 105.9), (263.8, 156.0), (263.8, 256.2), (350.6, 206.1)]);
    line(&[(5.0, 105.9), (178.5, 206.1), (178.5, 406.4)]);
    line(&[(177.0, 206.1), (263.8, 156.0)]);
    p.add(egui::Shape::closed_line(
        m(&[
            (178.5, 5.8),
            (5.0, 105.9),
            (5.0, 206.1),
            (91.7, 256.2),
            (91.7, 356.4),
            (178.5, 406.4),
            (352.0, 306.3),
            (352.0, 206.1),
            (265.2, 156.0),
            (265.2, 55.9),
        ]),
        edge,
    ));
}

/// `scale`(1.0 = 96 DPI)下的缩放边框物理像素,向下不小于 1px。
pub fn resize_border_px(scale: f32) -> f32 {
    let s = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    (RESIZE_BORDER_AT_96_DPI * s).round().max(1.0)
}

/// 角区边长:边宽的两倍,角容易点到。
fn corner_size(edge: f32) -> f32 {
    2.0 * edge
}

/// 窗口 `w` 的边/角区(zone, 方向, 光标)。角在后,重叠处角优先。
fn zones(w: Rect, edge: f32) -> [(Rect, ResizeDirection, CursorIcon); 8] {
    use ResizeDirection::*;
    let (e, c) = (edge, corner_size(edge));
    [
        (Rect::from_min_max(w.left_top(), pos2(w.right(), w.top() + e)), North, CursorIcon::ResizeVertical),
        (Rect::from_min_max(pos2(w.left(), w.bottom() - e), w.right_bottom()), South, CursorIcon::ResizeVertical),
        (Rect::from_min_max(w.left_top(), pos2(w.left() + e, w.bottom())), West, CursorIcon::ResizeHorizontal),
        (Rect::from_min_max(pos2(w.right() - e, w.top()), w.right_bottom()), East, CursorIcon::ResizeHorizontal),
        (Rect::from_min_size(w.left_top(), vec2(c, c)), NorthWest, CursorIcon::ResizeNwSe),
        (Rect::from_min_size(w.right_top() - vec2(c, 0.0), vec2(c, c)), NorthEast, CursorIcon::ResizeNeSw),
        (Rect::from_min_size(w.left_bottom() - vec2(0.0, c), vec2(c, c)), SouthWest, CursorIcon::ResizeNeSw),
        (Rect::from_min_size(w.right_bottom() - vec2(c, c), vec2(c, c)), SouthEast, CursorIcon::ResizeNwSe),
    ]
}

/// 窗口边缘的隐形缩放区(最上层);最大化/全屏时不布。
pub fn resize_zones(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    if maximized(&ctx) || ctx.input(|i| i.viewport().fullscreen.unwrap_or(false)) {
        return;
    }
    let w = ctx.content_rect();
    if !w.is_positive() {
        return;
    }
    let ppp = ctx.pixels_per_point();
    let edge = resize_border_px(ctx.input(|i| i.viewport().native_pixels_per_point).unwrap_or(ppp)) / ppp;
    let layer = LayerId::new(Order::Foreground, Id::new("titlebar-resize"));
    let zui = &mut ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(w));
    for (i, (r, dir, cursor)) in zones(w, edge).into_iter().enumerate() {
        let resp = zui.interact(r, Id::new(("titlebar-resize", i)), Sense::drag()).on_hover_cursor(cursor);
        if resp.drag_started_by(PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
        }
    }
}

/// 应用栏上的一条 1px 竖分隔线。
pub fn app_bar_separator(p: &egui::Painter, x: f32, bar: Rect, color: Color32) {
    p.line_segment(
        [pos2(x, bar.top() + RADIUS_SM as f32), pos2(x, bar.bottom() - RADIUS_SM as f32)],
        Stroke::new(1.0, color),
    );
}
