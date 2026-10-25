//! 视口:`solidcraft_gpu::view::Renderer` 在 `egui_wgpu` paint callback 里画
//! `solidcraft_render::Scene`。胶水层照抄 SolidCraft 的 `ui-egui/src/gpu.rs`。

use std::sync::{Arc, Mutex};

use eframe::egui_wgpu;
use egui_wgpu::wgpu;
use solidcraft_gpu::view::{Frame, GpuScene, GpuTarget, Renderer};
use solidcraft_render::Camera;

/// 一帧视口,作为 egui paint callback 提交。
pub struct ViewportCallback(pub Frame);

impl egui_wgpu::CallbackTrait for ViewportCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(r) = resources.get_mut::<Renderer>() {
            r.prepare(device, queue, encoder, &self.0);
        }
        Vec::new()
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(r) = resources.get::<Renderer>() else { return };
        let vp = info.viewport_in_pixels();
        r.paint(&self.0, pass, [vp.left_px as f32, vp.top_px as f32, vp.width_px as f32, vp.height_px as f32]);
    }
}

type SceneSlot = Arc<Mutex<Option<GpuScene>>>;

fn empty_slot() -> SceneSlot {
    Arc::new(Mutex::new(None))
}

pub struct Viewport {
    pub cam: Camera,
    /// 场景包围球半径(裁剪面与适配视图用)。
    pub radius: f64,
    slot: SceneSlot,
    key: u64,
    hl_slot: SceneSlot,
    hl_key: u64,
    /// 视口逻辑像素尺寸(最近一帧 show 的 rect),拾取用。
    pub size: [f32; 2],
    /// 断面:(世界法向, 平面距离 d),剖掉 n·p > d 一侧;None 关闭。
    pub section: Option<(solidcraft_geom::Vec3, f64)>,
    /// 断面 cap 是否画剖面填充线。
    pub hatch: bool,
    /// 渲染器是否已装进 egui 的 callback resources。
    pub gpu_ok: bool,
}

impl Default for Viewport {
    fn default() -> Self {
        Self::new()
    }
}

impl Viewport {
    pub fn new() -> Self {
        Self {
            cam: Camera::default(),
            radius: 1.0,
            slot: empty_slot(),
            key: 0,
            hl_slot: empty_slot(),
            hl_key: 0,
            size: [800.0, 600.0],
            section: None,
            hatch: true,
            gpu_ok: false,
        }
    }

    /// 用 eframe 的 wgpu 渲染状态创建管线:带深度/MSAA 的场景目标 + 用于回拷的无深度窗口目标。
    pub fn install(&mut self, rs: &eframe::egui_wgpu::RenderState) -> Result<(), String> {
        let samples = effective_samples(rs);
        let scene = GpuTarget {
            format: rs.target_format,
            depth: Some(egui_wgpu::depth_format_from_bits(24, 8).ok_or("no depth24-stencil8 format")?),
            samples,
            tess: false,
        };
        let window = GpuTarget { format: rs.target_format, depth: None, samples: 1, tess: false };
        let renderer = Renderer::with_window_target(&rs.device, scene, window, None);
        rs.renderer.write().callback_resources.insert(renderer);
        self.gpu_ok = true;
        Ok(())
    }

    /// 新场景内容(文档加载或显隐变化后调用):压成 GPU 场景并递增 key。
    pub fn set_scene(&mut self, scene: &solidcraft_render::Scene) {
        let gs = solidcraft_gpu::capture::scene_geometry(scene);
        if let Ok(mut slot) = self.slot.lock() {
            *slot = Some(gs);
        }
        self.key += 1;
    }

    /// 高亮场景(选中的面/体),None 清除。
    pub fn set_highlight(&mut self, highlight: Option<GpuScene>) {
        if let Ok(mut slot) = self.hl_slot.lock() {
            *slot = highlight;
        }
        self.hl_key += 1;
    }

    pub fn show(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        self.size = [rect.width(), rect.height()];
        if !self.gpu_ok {
            ui.painter().rect_filled(rect, 0.0, egui::Color32::from_rgb(30, 32, 36));
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "没有可用的 wgpu 渲染状态\n(尝试 --release 或检查显卡驱动)",
                egui::FontId::proportional(16.0),
                egui::Color32::from_gray(140),
            );
            return;
        }
        let ppp = ui.ctx().pixels_per_point();
        let aspect = f64::from(rect.width() / rect.height().max(1.0));
        let eye = self.cam.eye();
        let e = eye.to_f32();
        let frame = Frame {
            product: false,
            key: self.key,
            slot: Arc::clone(&self.slot),
            sketch_key: 0,
            sketch_slot: empty_slot(),
            hl_key: self.hl_key,
            hl_slot: Arc::clone(&self.hl_slot),
            pv_key: 0,
            pv_slot: empty_slot(),
            pv_hide: false,
            view_proj: self.cam.view_proj(aspect, self.radius).to_f32(),
            back: self.cam.back().to_f32(),
            size_px: [rect.width() * ppp, rect.height() * ppp],
            clip: match self.section {
                Some((n, d)) => [n.x as f32, n.y as f32, n.z as f32, d as f32],
                None => [0.0; 4],
            },
            cap: {
                let solidcraft_render::Rgb(r, g, b) = solidcraft_render::SECTION_CAP;
                [f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0, if self.hatch { 1.0 } else { 0.0 }]
            },
            images: Vec::new(),
            analysis: [0.0; 8],
            analysis_colors: [0.0; 36],
            access: None,
            outline: false,
            eye: [e[0], e[1], e[2], if self.cam.fov > 0.0 { 1.0 } else { 0.0 }],
            ao_radius: 0.0,
            ssaa: 1.0,
            hover_tint: [0.0; 3],
            selection_tint: [0.0; 3],
        };
        ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, ViewportCallback(frame)));
    }
}

fn effective_samples(rs: &eframe::egui_wgpu::RenderState) -> u32 {
    let requested = eframe::egui_wgpu::effective_msaa_samples(rs.adapter.get_info().backend, 4);
    if requested <= 1 || rs.msaa_supported.is_empty() || rs.msaa_supported.contains(&requested) {
        requested
    } else {
        1
    }
}
