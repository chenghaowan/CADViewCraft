//! 主窗口:Mayo 式布局(菜单栏 / 左侧模型树 / 中央视口 / 右侧属性 / 底部状态栏)。

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, channel};
use std::sync::Arc;
use std::thread;

use egui::{Color32, Sense};
use solidcraft_geom::Vec3;

use crate::scene::{Doc, Node, build_scene, load};
use crate::viewport::Viewport;

/// 当前选中:节点路径 + 可选的 B-rep 面号(None = 整体选中)。
#[derive(Clone, PartialEq)]
struct Sel {
    path: Vec<usize>,
    face: Option<u32>,
}

/// 选中反馈配色(Mayo 风格)。
const SEL_FACE: [u8; 4] = [255, 196, 64, 255];
const SEL_BODY: [u8; 4] = [96, 156, 232, 255];

/// 断面设置:轴向预设法向 + 剖切深度(0=不剖,1=全剖)。
#[derive(Clone, Copy, PartialEq)]
struct Section {
    normal: [f64; 3],
    cut: f64,
    hatch: bool,
}

/// 激活中的工具(测量或构造)。
#[derive(Clone, PartialEq)]
enum Tool {
    /// 点-点:收两个命中点。
    P2P(Vec<Vec3>),
    /// 体-体:收两个体路径。
    B2B(Vec<Vec<usize>>),
    /// 建点:每点一下出一个点(顶点/圆心自动吸附),Esc 退出。
    CPoint,
    /// 两点轴:收两个吸附点。
    CAxis2P(Vec<Vec3>),
    /// 边轴:点一条边(直边给方向,圆弧边给圆心+平面法向)。
    CAxisEdge,
    /// 圆柱轴:点一个圆柱/圆锥/圆环面。
    CAxisFace,
    /// 坐标系:先原点,再 Z 方向(直边/圆弧边/圆柱面;Enter=世界 Z)。
    CCsys(Option<Vec3>),
}

/// 已完成的测量结果(画线 + 属性面板)。
enum MeasureMsg {
    BodyDist(Vec<usize>, Vec<usize>, Option<DistData>),
    Mass(Vec<usize>, Option<MassData>),
    FaceInfo(Vec<usize>, u32, Option<FaceData>),
}

const MEASURE_COLOR: [u8; 4] = [120, 230, 140, 255];
/// 测量工具收集中已点体的平涂色(比选中色浅)。
const TOOL_PENDING: [u8; 4] = [150, 190, 240, 255];
/// 质心标记色。
const CENTROID_COLOR: [u8; 4] = [205, 130, 240, 255];
/// 构造点标记色。
const CONSTRUCT_POINT: [u8; 4] = [255, 170, 40, 255];
/// 构造轴线色。
const CONSTRUCT_AXIS: [u8; 4] = [255, 170, 40, 255];
/// 坐标系三轴(右手法则;与视口 gizmo 同色系)。
const CSYS_X: [u8; 4] = [214, 93, 93, 255];
const CSYS_Y: [u8; 4] = [120, 200, 110, 255];
const CSYS_Z: [u8; 4] = [110, 150, 235, 255];

/// 图标资源目录(编译期定位到项目 assets/,与运行目录无关)。
const ICON_DIR: &str = env!("CARGO_MANIFEST_DIR");

/// 图标 SVG 里 `currentColor` 注入的颜色:暗主题用亮墨,亮主题用深墨。
fn icon_ink(dark: bool) -> &'static str {
    if dark {
        "#c9d4e0"
    } else {
        "#3b4351"
    }
}

/// 全部图标字节(首次使用时读入;按暗/亮两套注入色各存一份)。
fn icon_bytes() -> &'static std::collections::HashMap<(bool, String), Vec<u8>> {
    static CACHE: std::sync::OnceLock<std::collections::HashMap<(bool, String), Vec<u8>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| {
        let sep: char = std::path::MAIN_SEPARATOR;
        let dir = format!("{ICON_DIR}{sep}assets{sep}icons");
        let mut map = std::collections::HashMap::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let path = e.path();
                if path.extension().is_some_and(|x| x == "svg")
                    && let Ok(text) = std::fs::read_to_string(&path)
                {
                    let stem = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
                    for dark in [true, false] {
                        map.insert(
                            (dark, stem.clone()),
                            text.replace("currentColor", icon_ink(dark)).into_bytes(),
                        );
                    }
                }
            }
        }
        map
    })
}

/// 图标(字节直供,不经 file loader;dark 决定注入墨色)。
fn icon_image(name: &str, size: f32, dark: bool) -> egui::Image<'static> {
    let fallback: &[u8] = b"<svg xmlns='http://www.w3.org/2000/svg'/>";
    let bytes = icon_bytes()
        .get(&(dark, name.to_string()))
        .cloned()
        .unwrap_or_else(|| fallback.to_vec());
    egui::Image::from_bytes(format!("cache://icons/{name}.svg"), bytes)
        .fit_to_exact_size(egui::vec2(size, size))
}
/// 质量属性:(体积 mm³, 表面积 mm², 质心, 包围盒尺寸)。
type MassData = (f64, f64, Vec3, [f64; 3]);
/// 面信息:(面积 mm², 质心, 平面法向)。
type FaceData = (f64, Vec3, Option<Vec3>);
/// 体-体距离:(距离, from, to, 是否精确)。
type DistData = (f64, Vec3, Vec3, bool);
/// 一次体-体测量:(两个体的路径, 结果;None = 计算中)。
type B2BEntry = ((Vec<usize>, Vec<usize>), Option<DistData>);

pub struct CadViewApp {
    vp: Viewport,
    doc: Option<Doc>,
    scene_dirty: bool,
    sel: Option<Sel>,
    section: Option<Section>,
    tool: Option<Tool>,
    /// 点-点测量结果。
    p2p: Option<(Vec3, Vec3)>,
    /// 体-体测量结果(后台完成前为 pending)。
    b2b: Option<B2BEntry>,
    /// 体质量属性缓存(选中时懒加载)。
    mass: HashMap<Vec<usize>, Option<MassData>>,
    /// 面信息缓存。
    face_info: HashMap<(Vec<usize>, u32), Option<FaceData>>,
    mrx: Option<Receiver<MeasureMsg>>,
    /// 最近打开的文件(新→旧,持久化)。
    recent: Vec<String>,
    /// 加载完成后下一次 refresh_scene 是否重新适配视图。
    fit_pending: bool,
    /// 右侧属性面板是否显示。
    show_props: bool,
    /// 是否在选中体上显示质心标记。
    show_centroid: bool,
    /// 设置浮窗是否打开。
    show_settings: bool,
    /// 断面浮窗是否打开(浮窗开着 = 断面生效)。
    show_section_dialog: bool,
    /// 亮/暗主题(None = 跟随系统)。
    dark_theme: Option<bool>,
    /// 旋转/平移的鼠标键(持久化)。
    mouse_rotate: u8,
    mouse_pan: u8,
    rx: Option<Receiver<LoadMsg>>,
    loading: Option<String>,
    error: Option<String>,
    status: String,
    /// 构造元素(点/轴/坐标系),换文件时清空。
    constructs: Vec<crate::construct::Construct>,
}

enum LoadMsg {
    Done(String, Result<Doc, String>),
}

impl CadViewApp {
    pub fn with_viewport(
        vp: Viewport,
        recent: Vec<String>,
        dark_theme: Option<bool>,
        mouse_rotate: u8,
        mouse_pan: u8,
    ) -> Self {
        Self {
            vp,
            doc: None,
            scene_dirty: false,
            sel: None,
            section: None,
            tool: None,
            p2p: None,
            b2b: None,
            mass: HashMap::new(),
            face_info: HashMap::new(),
            mrx: None,
            recent,
            fit_pending: false,
            show_props: false,
            show_centroid: false,
            show_settings: false,
            show_section_dialog: false,
            dark_theme,
            mouse_rotate,
            mouse_pan,
            rx: None,
            loading: None,
            error: None,
            status: "就绪".into(),
            constructs: Vec::new(),
        }
    }

    /// 启动参数里带的文件(创建后调用一次)。
    pub fn open_on_start(&mut self, path: &str) {
        self.start_load(path.to_string());
    }

    /// 后台线程加载;UI 每帧只收消息。
    fn start_load(&mut self, path: String) {
        let (tx, rx) = channel();
        self.rx = Some(rx);
        self.loading = Some(path.clone());
        self.error = None;
        self.status = format!("正在打开 {path} …");
        thread::spawn(move || {
            let result = load(&path);
            let _ = tx.send(LoadMsg::Done(path, result));
        });
    }

    fn poll_load(&mut self) {
        let Some(rx) = &self.rx else { return };
        match rx.try_recv() {
            Ok(LoadMsg::Done(path, result)) => {
                self.rx = None;
                self.loading = None;
                match result {
                    Ok(doc) => {
                        self.status = format!(
                            "{}  ·  {}  ·  {} 体 / {} 三角形 / 导入 {} ms",
                            path,
                            doc.schema.split_whitespace().next().unwrap_or("STEP"),
                            doc.bodies(),
                            doc.triangles(),
                            doc.load_ms
                        );
                        for w in &doc.warnings {
                            log::warn!("导入警告: {w}");
                        }
                        self.doc = Some(doc);
                        self.sel = None;
                        self.tool = None;
                        self.p2p = None;
                        self.b2b = None;
                        self.mass.clear();
                        self.face_info.clear();
                        self.constructs.clear();
                        self.push_recent(path.clone());
                        self.fit_pending = true;
                        self.scene_dirty = true;
                    }
                    Err(e) => {
                        self.error = Some(e);
                        self.status = format!("打开失败:{path}");
                    }
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.rx = None,
        }
    }

    /// 适配视图:相机对准模型包围球。
    fn fit_view(&mut self) {
        let Some(doc) = &self.doc else { return };
        let Some((min, max)) = doc.bbox() else { return };
        let d = max - min;
        let r = 0.5 * (d.x * d.x + d.y * d.y + d.z * d.z).sqrt();
        self.vp.radius = r.max(1.0);
        self.vp.cam.target = Vec3::new((min.x + max.x) * 0.5, (min.y + max.y) * 0.5, (min.z + max.z) * 0.5);
        self.vp.cam.distance = (r * 2.6).max(10.0);
    }

    /// 场景内容变化(加载完成/显隐切换)时重建渲染场景;视图只在加载后重适配。
    fn refresh_scene(&mut self) {
        let Some(doc) = &self.doc else { return };
        let sc = build_scene(doc);
        // 选中节点被隐藏就取消选中。
        if let Some(sel) = &self.sel
            && doc.node_visible(&sel.path) == Some(false)
        {
            self.sel = None;
        }
        if self.fit_pending {
            self.fit_view();
            self.fit_pending = false;
        }
        self.vp.set_scene(&sc);
        self.refresh_highlight();
        self.scene_dirty = false;
    }

    /// 弹出文件对话框并加载。
    fn open_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("CAD 文件", &["step", "stp", "igs", "iges"])
            .pick_file()
        {
            self.start_load(path.to_string_lossy().into_owned());
        }
    }

    /// 断面浮窗开关:浮窗开着 = 断面生效,关浮窗 = 关断面。
    fn toggle_section(&mut self) {
        self.show_section_dialog = !self.show_section_dialog;
        if self.show_section_dialog && self.section.is_none() {
            self.section = Some(Section { normal: [0.0, 0.0, -1.0], cut: 0.5, hatch: true });
        }
        if !self.show_section_dialog {
            self.section = None;
        }
        self.sync_section();
    }

    /// 记入最近文件(去重置顶,最多 10 条)。
    fn push_recent(&mut self, path: String) {
        self.recent.retain(|p| p != &path);
        self.recent.insert(0, path);
        self.recent.truncate(10);
    }

    /// 视口点击拾取;x/y 是相对视口左上角的逻辑像素。
    fn pick_hit(&self, x: f32, y: f32) -> Option<crate::pick::Hit> {
        let clip = self.section_clip();
        self.doc.as_ref().and_then(|doc| {
            crate::pick::pick(
                doc,
                &self.vp.cam,
                f64::from(x),
                f64::from(y),
                f64::from(self.vp.size[0]),
                f64::from(self.vp.size[1]),
                clip,
            )
        })
    }

    /// 当前断面为着色器 uniform 的 (法向, d);无断面或无模型返回 None。
    fn section_clip(&self) -> Option<(Vec3, f64)> {
        let sec = self.section?;
        let (min, max) = self.doc.as_ref()?.bbox()?;
        let n = Vec3::new(sec.normal[0], sec.normal[1], sec.normal[2]);
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for c in [
            [min.x, min.y, min.z],
            [max.x, min.y, min.z],
            [min.x, max.y, min.z],
            [min.x, min.y, max.z],
            [max.x, max.y, min.z],
            [max.x, min.y, max.z],
            [min.x, max.y, max.z],
            [max.x, max.y, max.z],
        ] {
            let t = n.dot(Vec3::new(c[0], c[1], c[2]));
            lo = lo.min(t);
            hi = hi.max(t);
        }
        // cut=0 → d=hi(不剖),cut=1 → d=lo(全剖);shader 剖掉 n·p > d 一侧。
        Some((n, hi - sec.cut * (hi - lo)))
    }

    /// 把断面状态同步到视口(uniform 级,无需重建场景)。
    fn sync_section(&mut self) {
        self.vp.section = self.section_clip();
        self.vp.hatch = self.section.is_some_and(|s| s.hatch);
    }

    // ---------------- 测量 ----------------

    /// 后台测量结果入队(每帧轮询)。
    fn poll_measure(&mut self) {
        let Some(rx) = &self.mrx else { return };
        match rx.try_recv() {
            Ok(MeasureMsg::BodyDist(a, b, r)) => {
                self.b2b = Some(((a, b), r));
                self.mrx = None;
            }
            Ok(MeasureMsg::Mass(path, r)) => {
                self.mass.insert(path, r);
            }
            Ok(MeasureMsg::FaceInfo(path, face, r)) => {
                self.face_info.insert((path, face), r);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.mrx = None,
        }
    }

    fn measure_node(&self, path: &[usize]) -> Option<(Arc<solidcraft_kernel::Body>, Arc<solidcraft_geom::Mesh>)> {
        let doc = self.doc.as_ref()?;
        let node = doc.node_at(path)?;
        Some((node.body.as_ref()?.clone(), node.mesh.as_ref()?.clone()))
    }

    /// 选中体的质量属性(后台 kernel::measure,大件需数秒)。
    fn request_mass(&mut self, path: Vec<usize>) {
        if self.mass.contains_key(&path) || self.mrx.is_some() {
            return;
        }
        let Some((body, _)) = self.measure_node(&path) else { return };
        let (tx, rx) = channel();
        self.mrx = Some(rx);
        thread::spawn(move || {
            let r = solidcraft_kernel::measure(&body).ok().map(|m| {
                (
                    m.volume,
                    m.area,
                    m.centroid,
                    [m.bbox.max.x - m.bbox.min.x, m.bbox.max.y - m.bbox.min.y, m.bbox.max.z - m.bbox.min.z],
                )
            });
            let _ = tx.send(MeasureMsg::Mass(path, r));
        });
    }

    /// 选中面的面积/质心/法向(后台)。
    fn request_face(&mut self, path: Vec<usize>, face: u32) {
        let key = (path.clone(), face);
        if self.face_info.contains_key(&key) || self.mrx.is_some() {
            return;
        }
        let Some((body, mesh)) = self.measure_node(&path) else { return };
        let (tx, rx) = channel();
        self.mrx = Some(rx);
        thread::spawn(move || {
            let chord = body.size().max(1.0) * 1e-3;
            let r = body
                .faces(chord)
                .ok()
                .and_then(|faces| {
                    faces
                        .into_iter()
                        .find(|f| f.index == face as usize)
                        .map(|f| (f.area, f.centroid, f.plane_normal))
                });
            drop(mesh);
            let _ = tx.send(MeasureMsg::FaceInfo(path, face, r));
        });
    }

    /// 体-体最小距离(后台 kernel::body_distance)。
    fn request_body_dist(&mut self, a: Vec<usize>, b: Vec<usize>) {
        let Some((body_a, mesh_a)) = self.measure_node(&a) else { return };
        let Some((body_b, mesh_b)) = self.measure_node(&b) else { return };
        let (tx, rx) = channel();
        self.mrx = Some(rx);
        self.b2b = Some(((a.clone(), b.clone()), None));
        thread::spawn(move || {
            let chord_a = body_a.size().max(1.0) * 1e-3;
            let chord_b = body_b.size().max(1.0) * 1e-3;
            let sa = solidcraft_kernel::DistanceShape::new(&body_a, &mesh_a, chord_a);
            let sb = solidcraft_kernel::DistanceShape::new(&body_b, &mesh_b, chord_b);
            let limit = body_a.size() + body_b.size();
            let r = solidcraft_kernel::body_distance(&sa, &sb, limit)
                .ok()
                .flatten()
                .map(|d| (d.distance, d.from, d.to, d.exact));
            let _ = tx.send(MeasureMsg::BodyDist(a, b, r));
        });
    }

    /// 构造轴的半长:随场景尺度(包围球半径的 1/4)。
    fn axis_len(&self) -> f64 {
        self.vp.radius.max(1.0) * 0.25
    }

    /// 吸附拾取一个点(构造工具的输入):顶点 → 圆弧边圆心 → 圆类面圆心 → 面命中点。
    /// x/y 是相对视口左上角的逻辑像素。
    fn snap_point(&self, x: f32, y: f32, hit: Option<&crate::pick::Hit>) -> Option<Vec3> {
        let doc = self.doc.as_ref()?;
        let (w, h) = (f64::from(self.vp.size[0]), f64::from(self.vp.size[1]));
        if let Some(v) = crate::pick::pick_vertex(doc, &self.vp.cam, f64::from(x), f64::from(y), w, h, self.vp.radius, 9.0) {
            return Some(v.point);
        }
        if let Some(e) = crate::pick::pick_edge(doc, &self.vp.cam, f64::from(x), f64::from(y), w, h, self.vp.radius, 6.0)
            && let Some((center, _)) = crate::construct::circle_edge(doc, &e.path, e.edge)
        {
            return Some(center);
        }
        if let Some(h) = hit
            && let Some(c) = crate::construct::round_face_center(doc, &h.path, h.face)
        {
            return Some(c);
        }
        hit.map(|h| h.point)
    }

    /// 工具激活时吃掉一次点击;返回 true 表示已消费。x/y 是相对视口左上角的逻辑像素。
    fn tool_click(&mut self, hit: Option<&crate::pick::Hit>, x: f32, y: f32) -> bool {
        let Some(tool) = self.tool.clone() else { return false };
        match tool {
            Tool::P2P(mut pts) => {
                if let Some(h) = hit {
                    pts.push(h.point);
                    if pts.len() == 2 {
                        let [a, b] = [pts[0], pts[1]];
                        self.p2p = Some((a, b));
                        self.tool = None;
                        self.status = format!("点距:{:.3} mm", (b - a).len());
                    } else {
                        self.status = "点-点测距:再点一个点".into();
                        self.tool = Some(Tool::P2P(pts));
                    }
                    self.refresh_highlight();
                }
                true
            }
            Tool::B2B(mut bodies) => {
                if let Some(h) = hit {
                    if bodies.contains(&h.path) {
                        return true;
                    }
                    bodies.push(h.path.clone());
                    if bodies.len() == 2 {
                        let [a, b] = [bodies[0].clone(), bodies[1].clone()];
                        self.tool = None;
                        self.request_body_dist(a, b);
                        self.status = "体-体测距:计算中…".into();
                    } else {
                        self.status = "体-体测距:再点一个体".into();
                        self.tool = Some(Tool::B2B(bodies));
                    }
                    self.refresh_highlight();
                }
                true
            }
            Tool::CPoint => {
                if let Some(p) = self.snap_point(x, y, hit) {
                    let name = crate::construct::next_name(&self.constructs, "P");
                    self.status = format!("{name}:({:.2}, {:.2}, {:.2})(Esc 退出)", p.x, p.y, p.z);
                    self.constructs.push(crate::construct::Construct::Point { name, p });
                    self.refresh_highlight();
                } else {
                    self.status = "没点到模型;点顶点/圆弧/面上".into();
                }
                true
            }
            Tool::CAxis2P(pts) => {
                let mut pts = pts;
                if let Some(p) = self.snap_point(x, y, hit) {
                    pts.push(p);
                    if pts.len() == 2 {
                        let (a, b) = (pts[0], pts[1]);
                        match (b - a).normalized() {
                            Some(dir) => {
                                let name = crate::construct::next_name(&self.constructs, "A");
                                self.status = format!("{name}:两点轴");
                                self.constructs.push(crate::construct::Construct::Axis {
                                    name,
                                    origin: a,
                                    dir,
                                    len: self.axis_len(),
                                });
                                self.tool = None;
                            }
                            None => self.status = "两点重合,轴无法建立".into(),
                        }
                    } else {
                        self.status = "两点轴:再点一个点".into();
                    }
                    self.tool = Some(Tool::CAxis2P(pts));
                    self.refresh_highlight();
                }
                true
            }
            Tool::CAxisEdge => {
                let doc = self.doc.as_ref();
                if let Some(doc) = doc {
                    let (w, h) = (f64::from(self.vp.size[0]), f64::from(self.vp.size[1]));
                    if let Some(e) = crate::pick::pick_edge(doc, &self.vp.cam, f64::from(x), f64::from(y), w, h, self.vp.radius, 6.0) {
                        let made = if let Some((a, dir)) = crate::construct::straight_edge(&e.polyline) {
                            Some((a, dir, "直边".to_string()))
                        } else {
                            crate::construct::circle_edge(doc, &e.path, e.edge).map(|(c, n)| (c, n, "圆弧法向".to_string()))
                        };
                        match made {
                            Some((origin, dir, how)) => {
                                let name = crate::construct::next_name(&self.constructs, "A");
                                self.status = format!("{name}:{how}轴");
                                self.constructs.push(crate::construct::Construct::Axis { name, origin, dir, len: self.axis_len() });
                                self.tool = None;
                                self.refresh_highlight();
                            }
                            None => self.status = "这条边不是直线也不是圆弧".into(),
                        }
                    } else {
                        self.status = "没点到边;点模型的棱线".into();
                    }
                }
                true
            }
            Tool::CAxisFace => {
                if let Some(h) = &hit {
                    let doc = self.doc.as_ref();
                    match doc.and_then(|d| crate::construct::round_face_axis(d, &h.path, h.face)) {
                        Some((origin, dir)) => {
                            let name = crate::construct::next_name(&self.constructs, "A");
                            self.status = format!("{name}:圆柱/圆锥/圆环轴");
                            self.constructs.push(crate::construct::Construct::Axis { name, origin, dir, len: self.axis_len() });
                            self.tool = None;
                            self.refresh_highlight();
                        }
                        None => self.status = "这个面不是圆柱/圆锥/圆环面".into(),
                    }
                } else {
                    self.status = "没点到模型;点一个圆类面".into();
                }
                true
            }
            Tool::CCsys(origin) => match origin {
                None => {
                    if let Some(o) = self.snap_point(x, y, hit) {
                        self.status = "坐标系:再点 Z 方向(直边/圆弧/圆柱面),Enter=世界 Z".into();
                        self.tool = Some(Tool::CCsys(Some(o)));
                        self.refresh_highlight();
                    } else {
                        self.status = "没点到模型;点原点(顶点/圆心/任意点)".into();
                    }
                    true
                }
                Some(o) => {
                    if let Some(z) = self.pick_direction(x, y, hit) {
                        self.finish_csys(o, z);
                    } else {
                        self.status = "没拾取到方向;点直边/圆弧/圆柱面,或 Enter 用世界 Z".into();
                    }
                    true
                }
            },
        }
    }

    /// 方向拾取(坐标系 Z):直边方向 / 圆弧边法向 / 圆柱类面轴向。
    fn pick_direction(&self, x: f32, y: f32, hit: Option<&crate::pick::Hit>) -> Option<Vec3> {
        let doc = self.doc.as_ref()?;
        let (w, h) = (f64::from(self.vp.size[0]), f64::from(self.vp.size[1]));
        if let Some(e) = crate::pick::pick_edge(doc, &self.vp.cam, f64::from(x), f64::from(y), w, h, self.vp.radius, 6.0) {
            if let Some((_, dir)) = crate::construct::straight_edge(&e.polyline) {
                return Some(dir);
            }
            if let Some((_, axis)) = crate::construct::circle_edge(doc, &e.path, e.edge) {
                return Some(axis);
            }
        }
        if let Some(h) = hit
            && let Some((_, axis)) = crate::construct::round_face_axis(doc, &h.path, h.face)
        {
            return Some(axis);
        }
        None
    }

    /// 用原点与 Z 方向落一个坐标系(X = 世界 X 投影,照 SolidCraft)。
    fn finish_csys(&mut self, origin: Vec3, z: Vec3) {
        match crate::construct::csys_axes(z) {
            Some((x, y)) => {
                let name = crate::construct::next_name(&self.constructs, "C");
                self.status = format!("{name}:坐标系");
                self.constructs.push(crate::construct::Construct::Csys { name, origin, x, y, z });
                self.tool = None;
            }
            None => self.status = "Z 方向退化,无法建立坐标系".into(),
        }
        self.refresh_highlight();
    }

    fn set_sel(&mut self, sel: Option<Sel>) {
        self.sel = sel;
        self.refresh_highlight();
    }

    /// 把选中反馈(面平涂/体平涂)压成高亮场景。
    fn refresh_highlight(&mut self) {
        let Some(scene) = self.highlight_scene() else {
            self.vp.set_highlight(None);
            return;
        };
        self.vp.set_highlight(Some(scene));
    }

    fn highlight_scene(&self) -> Option<solidcraft_gpu::view::GpuScene> {
        let doc = self.doc.as_ref()?;
        let mut sc = solidcraft_gpu::view::GpuScene::default();
        if let Some(sel) = &self.sel
            && let Some(node) = doc.node_at(&sel.path)
            && let Some(mesh) = &node.mesh
        {
            let (color, lit) = if sel.face.is_some() {
                (SEL_FACE, false)
            } else {
                (SEL_BODY, true)
            };
            for (tri, face) in mesh.triangles.iter().zip(&mesh.tri_face) {
                let takes = sel.face.is_none_or(|f| *face == f);
                if !takes {
                    continue;
                }
                for k in tri {
                    let i = *k as usize;
                    if let (Some(p), Some(n)) = (mesh.positions.get(i), mesh.normals.get(i)) {
                        let normal = if lit { n.to_f32() } else { [0.0; 3] };
                        sc.tri(p.to_f32(), normal, color);
                    }
                }
            }
        }
        // 测量图形:端点十字 + 连线(画在最上层)。
        let mark = |sc: &mut solidcraft_gpu::view::GpuScene, p: Vec3, col: [u8; 4]| {
            // 十字大小随缩放保持约 9 像素:每像素的世界尺寸 × 9。
            let px_world = 2.0 * self.vp.cam.half_height() / f64::from(self.vp.size[1].max(1.0));
            let r = px_world * 9.0;
            for d in [Vec3::X, Vec3::Y, Vec3::Z] {
                sc.line((p - d * r).to_f32(), (p + d * r).to_f32(), col, 1.5, true);
            }
        };
        // 工具进行中:已拾取的点/体的即时反馈。
        match &self.tool {
            Some(Tool::P2P(pts)) => {
                for pt in pts {
                    mark(&mut sc, *pt, MEASURE_COLOR);
                }
            }
            Some(Tool::CAxis2P(pts)) => {
                for pt in pts {
                    mark(&mut sc, *pt, CONSTRUCT_POINT);
                }
            }
            Some(Tool::CCsys(Some(o))) => {
                mark(&mut sc, *o, CONSTRUCT_POINT);
            }
            Some(Tool::B2B(bodies)) => {
                for path in bodies {
                    if let Some(node) = doc.node_at(path)
                        && let Some(mesh) = &node.mesh
                    {
                        for (tri, _) in mesh.triangles.iter().zip(&mesh.tri_face) {
                            for k in tri {
                                if let Some(p) = mesh.positions.get(*k as usize) {
                                    sc.tri(p.to_f32(), [0.0; 3], TOOL_PENDING);
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        if let Some((a, b)) = &self.p2p {
            mark(&mut sc, *a, MEASURE_COLOR);
            mark(&mut sc, *b, MEASURE_COLOR);
            sc.line(a.to_f32(), b.to_f32(), MEASURE_COLOR, 2.0, true);
        }
        if let Some((_, Some((_, from, to, _)))) = &self.b2b {
            mark(&mut sc, *from, MEASURE_COLOR);
            mark(&mut sc, *to, MEASURE_COLOR);
            sc.line(from.to_f32(), to.to_f32(), MEASURE_COLOR, 2.0, true);
        }
        // 质心标记(开启且选中体的质量属性已算出)。
        if self.show_centroid
            && let Some(sel) = &self.sel
            && sel.face.is_none()
            && let Some(Some((_, _, c, _))) = self.mass.get(&sel.path)
        {
            mark(&mut sc, *c, CENTROID_COLOR);
        }
        // 构造元素:点十字 / 轴线 / 三色坐标系(画在最上层)。
        let arm = self.axis_len();
        for c in &self.constructs {
            match c {
                crate::construct::Construct::Point { p, .. } => mark(&mut sc, *p, CONSTRUCT_POINT),
                crate::construct::Construct::Axis { origin, dir, len, .. } => {
                    let (a, b) = (*origin - *dir * *len, *origin + *dir * *len);
                    sc.line(a.to_f32(), b.to_f32(), CONSTRUCT_AXIS, 2.0, true);
                    mark(&mut sc, *origin, CONSTRUCT_AXIS);
                }
                crate::construct::Construct::Csys { origin, x, y, z, .. } => {
                    for (d, col) in [(*x, CSYS_X), (*y, CSYS_Y), (*z, CSYS_Z)] {
                        let e = *origin + d * arm;
                        sc.line(origin.to_f32(), e.to_f32(), col, 2.0, true);
                    }
                    mark(&mut sc, *origin, CONSTRUCT_POINT);
                }
            }
        }
        Some(sc)
    }

    /// 屏幕叠加标签(测量结果/质心/构造元素),返回 (世界点, 文本, 颜色)。
    fn overlay_labels(&self) -> Vec<(Vec3, String, Color32)> {
        let measure = Color32::from_rgb(120, 230, 140);
        let construct = Color32::from_rgb(255, 190, 90);
        let mut out = Vec::new();
        if let Some((a, b)) = &self.p2p {
            out.push(((*a + *b) * 0.5, format!("{:.3} mm", (*b - *a).len()), measure));
        }
        if let Some((_, Some((d, from, to, exact)))) = &self.b2b {
            let approx = if *exact { "" } else { " ≈" };
            out.push(((*from + *to) * 0.5, format!("{d:.3} mm{approx}"), measure));
        }
        if self.show_centroid
            && let Some(sel) = &self.sel
            && sel.face.is_none()
            && let Some(Some((_, _, c, _))) = self.mass.get(&sel.path)
        {
            out.push((*c, format!("质心 ({:.1}, {:.1}, {:.1})", c.x, c.y, c.z), measure));
        }
        for c in &self.constructs {
            use crate::construct::Construct;
            let (text, at) = match c {
                Construct::Point { name, p } => (format!("{name} ({:.1}, {:.1}, {:.1})", p.x, p.y, p.z), *p),
                Construct::Axis { name, origin, dir, len, .. } => (name.clone(), *origin + *dir * *len),
                Construct::Csys { name, origin, .. } => (name.clone(), *origin),
            };
            out.push((at, text, construct));
        }
        out
    }

    /// 视口右上角的坐标轴指示(世界 X/Y/Z 投影到屏幕平面,照 SolidCraft)。
    fn draw_axes_gizmo(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        use solidcraft_geom::Vec3 as _V3;
        let center = egui::pos2(rect.right() - 52.0, rect.top() + 52.0);
        let (right, up, _) = self.vp.cam.basis();
        let project = |v: _V3| egui::vec2(
            (v.dot(right)) as f32,
            -(v.dot(up)) as f32,
        );
        let len = 26.0;
        for (dir, label, color) in [
            (_V3::X, "X", Color32::from_rgb(214, 93, 93)),
            (_V3::Y, "Y", Color32::from_rgb(120, 200, 110)),
            (_V3::Z, "Z", Color32::from_rgb(110, 150, 235)),
        ] {
            let d = project(dir);
            let end = center + d * len;
            ui.painter()
                .line_segment([center, end], egui::Stroke::new(2.0, color));
            ui.painter().text(
                end + d * 9.0,
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::monospace(11.0),
                color,
            );
        }
        ui.painter().circle_filled(center, 2.5, Color32::from_gray(150));
    }

    fn handle_drop(&mut self, ui: &mut egui::Ui) {
        let dropped: Vec<String> = ui
            .ctx()
            .input(|i| i.raw.dropped_files.clone())
            .into_iter()
            .map(|f| f.path().to_string_lossy().into_owned())
            .collect();
        if let Some(path) = dropped.last() {
            self.start_load(path.clone());
        }
    }

    fn handle_input(&mut self, response: &egui::Response) {
        if response.dragged() {
            let d = response.drag_delta();
            let btn = pointer_button(self.mouse_rotate);
            let pan = pointer_button(self.mouse_pan);
            if response.dragged_by(btn) {
                self.vp.cam.yaw += f64::from(d.x) * 0.01;
                self.vp.cam.pitch =
                    (self.vp.cam.pitch + f64::from(d.y) * 0.01).clamp(-1.5533, 1.5533);
            } else if response.dragged_by(pan) {
                let (right, up, _) = self.vp.cam.basis();
                let s = self.vp.cam.distance * 0.0015;
                self.vp.cam.target -= right * (f64::from(d.x) * s);
                self.vp.cam.target += up * (f64::from(d.y) * s);
            }
        }
        if response.hovered() {
            let scroll = response.ctx.input(|i| i.smooth_scroll_delta().y);
            if scroll != 0.0 {
                self.vp.cam.distance =
                    (self.vp.cam.distance * (-f64::from(scroll) * 0.001).exp()).clamp(1.0, 1e9);
            }
        }
    }

    /// SolidCraft 式 ribbon:文件 / 视图 / 检查(测量)三组,图标在上、文字在下。
    fn draw_toolbar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("ribbon").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ribbon_group(ui, "文件", |ui| {
                    let menu = egui::menu::MenuButton::from_button(
                        egui::Button::image_and_text(icon_image("open", 20.0, matches!(ui.ctx().theme(), egui::Theme::Dark)), "文件"),
                    );
                    menu.ui(ui, |ui| {
                        if ui.button("打开…").clicked() {
                            self.open_dialog();
                        }
                        if !self.recent.is_empty() {
                            ui.separator();
                            let items = self.recent.clone();
                            for rp in items {
                                let name = std::path::Path::new(&rp)
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| rp.clone());
                                if ui.button(name).clicked() {
                                    self.start_load(rp);
                                }
                            }
                        }
                        ui.separator();
                        if ui.button("退出").clicked() {
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                });
                ribbon_group(ui, "视图", |ui| {
                    if self.ribbon_icon_button(ui, "fit", "适配", false).clicked() {
                        self.fit_view();
                    }
                    if self.ribbon_icon_button(ui, "lookat", "前视", false).clicked() {
                        let (yaw, pitch) = solidcraft_render::StandardView::Front.angles();
                        self.vp.cam.yaw = yaw;
                        self.vp.cam.pitch = pitch;
                    }
                    if self.ribbon_icon_button(ui, "iso_analysis", "等轴测", false).clicked() {
                        let (yaw, pitch) = solidcraft_render::StandardView::Iso.angles();
                        self.vp.cam.yaw = yaw;
                        self.vp.cam.pitch = pitch;
                    }
                });
                ribbon_group(ui, "检查", |ui| {
                    let p2p_on = matches!(self.tool, Some(Tool::P2P(_)));
                    if self
                        .ribbon_icon_button(ui, "measure", "点距", p2p_on)
                        .on_hover_text("测量两点距离")
                        .clicked()
                    {
                        if p2p_on {
                            self.tool = None;
                            self.refresh_highlight();
                        } else {
                            self.start_tool_p2p();
                        }
                    }
                    let b2b_on = matches!(self.tool, Some(Tool::B2B(_)));
                    if self
                        .ribbon_icon_button(ui, "inspect", "体距", b2b_on)
                        .on_hover_text("测量两实体最小距离")
                        .clicked()
                    {
                        if b2b_on {
                            self.tool = None;
                            self.refresh_highlight();
                        } else {
                            self.start_tool_b2b();
                        }
                    }
                    if self
                        .ribbon_icon_button(ui, "center_of_mass", "质心", self.show_centroid)
                        .on_hover_text("显示选中实体的质心标记")
                        .clicked()
                    {
                        self.show_centroid = !self.show_centroid;
                        self.refresh_highlight();
                    }
                    if self
                        .ribbon_icon_button(ui, "appearance", "属性", self.show_props)
                        .on_hover_text("开关右侧属性面板")
                        .clicked()
                    {
                        self.show_props = !self.show_props;
                    }
                    let sec_on = self.section.is_some();
                    if self
                        .ribbon_icon_button(ui, "section", "断面", sec_on)
                        .on_hover_text("开关剖切(参数见下方)")
                        .clicked()
                    {
                        self.toggle_section();
                    }
                });
                ribbon_group(ui, "构造", |ui| {
                    // 按变体比较选中态(收集中途参数变了也要保持高亮)。
                    let key = |t: &Tool| {
                        match t {
                            Tool::CPoint => 1,
                            Tool::CAxis2P(_) => 2,
                            Tool::CAxisEdge => 3,
                            Tool::CAxisFace => 4,
                            Tool::CCsys(_) => 5,
                            _ => 0,
                        }
                    };
                    let entries: &[(&str, &str, Tool, &str)] = &[
                        ("cpoint", "建点", Tool::CPoint, "构造点:吸附顶点/圆心,否则面命中点"),
                        ("caxis_2p", "两点轴", Tool::CAxis2P(Vec::new()), "过两点的构造轴"),
                        ("caxis_edge", "边轴", Tool::CAxisEdge, "直边方向 / 圆弧边圆心与法向"),
                        ("caxis_cyl", "圆柱轴", Tool::CAxisFace, "圆柱/圆锥/圆环面的轴线"),
                        ("ccsys", "坐标系", Tool::CCsys(None), "原点 + Z 方向(Enter=世界 Z)"),
                    ];
                    let mut pick: Option<Tool> = None;
                    let mut off = false;
                    for (icon, label, t, tip) in entries {
                        let sel = self.tool.as_ref().map(key) == Some(key(t));
                        if self.ribbon_icon_button(ui, icon, label, sel).on_hover_text(*tip).clicked() {
                            if sel {
                                off = true;
                            } else {
                                pick = Some(t.clone());
                            }
                        }
                    }
                    if let Some(t) = pick {
                        self.tool = Some(t);
                        self.status = match &self.tool {
                            Some(Tool::CPoint) => "建点:点顶点/圆弧/圆类面/任意点(Esc 退出)".into(),
                            Some(Tool::CAxis2P(_)) => "两点轴:点第一个点(Esc 取消)".into(),
                            Some(Tool::CAxisEdge) => "边轴:点一条直边或圆弧边(Esc 取消)".into(),
                            Some(Tool::CAxisFace) => "圆柱轴:点圆柱/圆锥/圆环面(Esc 取消)".into(),
                            Some(Tool::CCsys(_)) => "坐标系:点原点(Esc 取消)".into(),
                            _ => unreachable!(),
                        };
                        self.refresh_highlight();
                    } else if off {
                        self.tool = None;
                        self.status = "工具已取消".into();
                        self.refresh_highlight();
                    }
                });
            });
        });
        // 断面分析浮窗(照 SolidCraft Section Analysis)。
        if self.show_section_dialog {
            let mut open = true;
            egui::Window::new(egui::RichText::new("断面分析").strong())
                .open(&mut open)
                .resizable(false)
                .collapsible(false)
                .show(ui.ctx(), |ui| {
                    let mut sec = self.section;
                    if let Some(s) = &mut sec {
                        ui.horizontal(|ui| {
                            ui.label("方向").on_hover_text("剖切平面的法向");
                            for (label, n) in [
                                ("-Z", [0.0, 0.0, -1.0]),
                                ("+Z", [0.0, 0.0, 1.0]),
                                ("+X", [1.0, 0.0, 0.0]),
                                ("+Y", [0.0, 1.0, 0.0]),
                            ] {
                                if ui.selectable_label(s.normal == n, label).clicked() {
                                    s.normal = n;
                                }
                            }
                            if ui.button("反转").on_hover_text("法向取反").clicked() {
                                s.normal = [-s.normal[0], -s.normal[1], -s.normal[2]];
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("深度");
                            ui.add(
                                egui::Slider::new(&mut s.cut, 0.0..=1.0)
                                    .show_value(true)
                                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                            );
                        });
                        ui.checkbox(&mut s.hatch, "剖面填充线");
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("关闭断面").clicked() {
                            self.show_section_dialog = false;
                            self.section = None;
                            self.sync_section();
                        }
                        if ui.button("取消").clicked() {
                            self.show_section_dialog = false;
                        }
                    });
                    if sec != self.section {
                        self.section = sec;
                        self.sync_section();
                    }
                });
            if !open {
                self.show_section_dialog = false;
                self.section = None;
                self.sync_section();
            }
        }

        // 设置浮窗:主题 + 鼠标键位。
        if self.show_settings {
            let mut open = true;
            egui::Window::new(egui::RichText::new("设置").strong())
                .open(&mut open)
                .resizable(false)
                .collapsible(false)
                .show(ui.ctx(), |ui| {
                    ui.label("主题");
                    ui.horizontal(|ui| {
                        let dark_now = self.dark_theme.unwrap_or(matches!(ui.ctx().theme(), egui::Theme::Dark));
                        if ui.selectable_label(self.dark_theme.is_none(), "跟随系统").clicked() {
                            self.dark_theme = None;
                        }
                        if ui.selectable_label(dark_now && self.dark_theme.is_some(), "暗色").clicked() {
                            self.dark_theme = Some(true);
                        }
                        if ui.selectable_label(!dark_now && self.dark_theme.is_some(), "亮色").clicked() {
                            self.dark_theme = Some(false);
                        }
                    });
                    ui.separator();
                    ui.label("鼠标");
                    egui::Grid::new("mouse_cfg").num_columns(2).show(ui, |ui| {
                        let names = [(0u8, "左键"), (1u8, "中键"), (2u8, "右键")];
                        ui.label("旋转");
                        egui::ComboBox::from_id_salt("rotate_btn")
                            .selected_text(
                                names.iter().find(|(v, _)| *v == self.mouse_rotate).map(|(_, n)| *n).unwrap_or("?"),
                            )
                            .show_ui(ui, |ui| {
                                for (v, n) in names {
                                    ui.selectable_value(&mut self.mouse_rotate, v, n);
                                }
                            });
                        ui.end_row();
                        ui.label("平移");
                        egui::ComboBox::from_id_salt("pan_btn")
                            .selected_text(
                                names.iter().find(|(v, _)| *v == self.mouse_pan).map(|(_, n)| *n).unwrap_or("?"),
                            )
                            .show_ui(ui, |ui| {
                                for (v, n) in names {
                                    ui.selectable_value(&mut self.mouse_pan, v, n);
                                }
                            });
                        ui.end_row();
                    });
                    ui.weak("缩放固定为滚轮");
                });
            if !open {
                self.show_settings = false;
            }
        }
    }

    /// SolidCraft 式 ribbon 图标按钮:图标在上、文字在下,选中/悬停有底色。
    fn ribbon_icon_button(&self, ui: &mut egui::Ui, icon: &str, label: &str, selected: bool) -> egui::Response {
        let resp = ui
            .allocate_ui(egui::vec2(64.0, 54.0), |ui| {
                ui.vertical_centered(|ui| {
                    ui.add(icon_image(icon, 24.0, matches!(ui.ctx().theme(), egui::Theme::Dark)));
                    ui.label(egui::RichText::new(label).small());
                });
            })
            .response;
        let interact = ui.interact(resp.rect, ui.id().with(("ribbon", label)), egui::Sense::click());
        let visuals = ui.visuals();
        if selected {
            ui.painter().rect_filled(resp.rect.shrink(1.0), 4.0, visuals.selection.bg_fill);
            ui.painter().rect_stroke(resp.rect.shrink(1.0), 4.0, visuals.selection.stroke, egui::StrokeKind::Inside);
        } else if interact.hovered() {
            ui.painter().rect_filled(resp.rect.shrink(1.0), 4.0, visuals.widgets.hovered.bg_fill);
        }
        interact
    }

    fn start_tool_p2p(&mut self) {
        self.tool = Some(Tool::P2P(Vec::new()));
        self.p2p = None;
        self.status = "点-点测距:点第一个点(Esc 取消)".into();
        self.refresh_highlight();
    }

    fn start_tool_b2b(&mut self) {
        self.tool = Some(Tool::B2B(Vec::new()));
        self.b2b = None;
        self.status = "体-体测距:点第一个体(Esc 取消)".into();
        self.refresh_highlight();
    }

    /// 左侧模型树:checkbox 显隐;立即模式下先收集要切换的路径,再统一应用。
    fn draw_tree(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("model_tree")
            .resizable(true)
            .default_size(260.0)
            .show(ui, |ui| {
                ui.heading("模型");
                ui.separator();
                let Some(doc) = &self.doc else {
                    ui.weak("尚未打开文件");
                    ui.weak("拖入 STEP/IGES,或 ribbon → 文件 → 打开");
                    return;
                };
                let mut toggles: Vec<Vec<usize>> = Vec::new();
                let mut selects: Vec<Vec<usize>> = Vec::new();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (i, root) in doc.roots.iter().enumerate() {
                        tree_node_ui(ui, root, &[i], &mut toggles, &mut selects, self.sel.as_ref());
                    }
                });
                if !toggles.is_empty()
                    && let Some(doc) = &mut self.doc
                {
                    for path in toggles {
                        toggle(&mut doc.roots, &path);
                    }
                    self.scene_dirty = true;
                }
                if let Some(path) = selects.into_iter().next() {
                    self.set_sel(Some(Sel { path, face: None }));
                }
            });
    }

    fn draw_info(&mut self, ui: &mut egui::Ui) {
        if !self.show_props {
            return;
        }
        egui::Panel::right("info_panel")
            .resizable(true)
            .default_size(230.0)
            .show(ui, |ui| {
                ui.heading("属性");
                ui.separator();
                let Some(doc) = &self.doc else {
                    ui.weak("(无模型)");
                    return;
                };
                egui::Grid::new("props").num_columns(2).show(ui, |ui| {
                    ui.label("名称");
                    ui.label(&doc.name);
                    ui.end_row();
                    ui.label("格式");
                    ui.label(doc.schema.split_whitespace().next().unwrap_or("?"));
                    ui.end_row();
                    ui.label("实体");
                    ui.label(doc.bodies().to_string());
                    ui.end_row();
                    ui.label("三角形");
                    ui.label(doc.triangles().to_string());
                    ui.end_row();
                    if let Some((min, max)) = doc.bbox() {
                        ui.label("包围盒");
                        ui.label(format!(
                            "{:.1} × {:.1} × {:.1} mm",
                            max.x - min.x,
                            max.y - min.y,
                            max.z - min.z
                        ));
                        ui.end_row();
                    }
                });
                // 选中信息先取成本地值,让 doc 的借用在此结束。
                let sel_info = self.sel.as_ref().and_then(|sel| {
                    doc.node_at(&sel.path).map(|n| (n.name.clone(), sel.face))
                });
                ui.separator();
                if let Some((name, face)) = sel_info {
                    ui.heading("选中");
                    egui::Grid::new("sel").num_columns(2).show(ui, |ui| {
                        ui.label("对象");
                        ui.label(&name);
                        ui.end_row();
                        ui.label("类型");
                        ui.label(if face.is_some() { "面" } else { "体" });
                        ui.end_row();
                        if let Some(f) = face {
                            ui.label("面号");
                            ui.label(f.to_string());
                            ui.end_row();
                        }
                    });
                    // 测量结果(懒加载,后台计算)。
                    let sel_path = self.sel.as_ref().map(|s| s.path.clone());
                    if let Some(path) = sel_path {
                        match face {
                            Some(f) => match self.face_info.get(&(path, f)) {
                                Some(Some((area, centroid, normal))) => {
                                    ui.label("面积");
                                    ui.label(format!("{area:.2} mm²"));
                                    ui.end_row();
                                    if let Some(n) = normal {
                                        ui.label("法向");
                                        ui.label(format!("({:.2}, {:.2}, {:.2})", n.x, n.y, n.z));
                                        ui.end_row();
                                    }
                                    ui.label("质心");
                                    ui.label(format!("({:.2}, {:.2}, {:.2})", centroid.x, centroid.y, centroid.z));
                                    ui.end_row();
                                }
                                Some(None) => {
                                    ui.label("面积");
                                    ui.label("(无信息)");
                                    ui.end_row();
                                }
                                None => {
                                    ui.label("面积");
                                    ui.label("计算中…");
                                    ui.end_row();
                                }
                            },
                            None => match self.mass.get(&path) {
                                Some(Some((vol, area, centroid, dims))) => {
                                    ui.label("体积");
                                    ui.label(format!("{vol:.2} mm³"));
                                    ui.end_row();
                                    ui.label("表面积");
                                    ui.label(format!("{area:.2} mm²"));
                                    ui.end_row();
                                    ui.label("包围盒");
                                    ui.label(format!("{:.2} × {:.2} × {:.2} mm", dims[0], dims[1], dims[2]));
                                    ui.end_row();
                                    ui.label("质心");
                                    ui.label(format!("({:.2}, {:.2}, {:.2})", centroid.x, centroid.y, centroid.z));
                                    ui.end_row();
                                }
                                Some(None) => {
                                    ui.label("体积");
                                    ui.label("(无法测量)");
                                    ui.end_row();
                                }
                                None => {
                                    ui.label("体积");
                                    ui.label("计算中…");
                                    ui.end_row();
                                }
                            },
                        }
                    }
                    ui.separator();
                }
            });
    }

    fn draw_status(&self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                if self.loading.is_some() {
                    ui.spinner();
                }
                if let Some(e) = &self.error {
                    ui.colored_label(Color32::from_rgb(230, 120, 110), format!("错误:{e}"));
                    ui.separator();
                }
                ui.label(&self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.weak("单击选面 · 双击选体 · 测量:菜单或拖入文件 · 左键旋转 · 滚轮缩放");
                });
            });
        });
    }
}

/// 树节点行:checkbox 显隐、名字可点选整个体;收集切换/选中的路径。
fn tree_node_ui(
    ui: &mut egui::Ui,
    node: &Node,
    path: &[usize],
    toggles: &mut Vec<Vec<usize>>,
    selects: &mut Vec<Vec<usize>>,
    sel: Option<&Sel>,
) {
    let id = format!("node{}", path.iter().fold(0usize, |a, b| a * 1000 + b));
    ui.push_id(id, |ui| {
        ui.horizontal(|ui| {
            // 眼睛按钮(可见=亮,隐藏=暗):点击切换显隐。
            let eye = ui.add(icon_image("eye", 15.0, matches!(ui.ctx().theme(), egui::Theme::Dark)).tint(if node.visible {
                Color32::from_gray(200)
            } else {
                Color32::from_gray(70)
            }));
            if eye.clicked() {
                toggles.push(path.to_vec());
            }
            // 体/装配图标。
            let kind = if node.mesh.is_some() { "body" } else { "component" };
            ui.add(icon_image(kind, 15.0, matches!(ui.ctx().theme(), egui::Theme::Dark)));
            let selected = sel.is_some_and(|s| s.path == path);
            let mut name = egui::RichText::new(&node.name);
            if node.mesh.is_none() {
                name = name.weak();
            }
            if selected {
                name = name.strong().color(Color32::from_rgb(255, 200, 90));
            }
            if ui
                .selectable_label(selected, name)
                .clicked()
                && node.mesh.is_some()
            {
                selects.push(path.to_vec());
            }
        });
        if !node.children.is_empty() {
            egui::CollapsingHeader::new("子件")
                .default_open(true)
                .show(ui, |ui| {
                    for (i, child) in node.children.iter().enumerate() {
                        let mut p = path.to_vec();
                        p.push(i);
                        tree_node_ui(ui, child, &p, toggles, selects, sel);
                    }
                });
        }
    });
}

/// ribbon 分组:按钮行 + 下方组名(照 SolidCraft 的 Measure/Section 分组样式)。
/// 注意:不能用 vertical_centered——它会把组宽撑到剩余全宽,把后续组挤出屏幕。
fn ribbon_group(ui: &mut egui::Ui, caption: &str, content: impl FnOnce(&mut egui::Ui)) {
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            content(ui);
        });
        ui.weak(egui::RichText::new(caption).small());
    });
    ui.separator();
}

/// u8 → 鼠标键(0 左键 / 1 中键 / 2 右键;越界回退左键)。
fn pointer_button(v: u8) -> egui::PointerButton {
    match v {
        1 => egui::PointerButton::Middle,
        2 => egui::PointerButton::Secondary,
        _ => egui::PointerButton::Primary,
    }
}

fn toggle(nodes: &mut [Node], path: &[usize]) {
    let Some((first, rest)) = path.split_first() else { return };
    let Some(node) = nodes.get_mut(*first) else { return };
    if rest.is_empty() {
        node.visible = !node.visible;
    } else {
        toggle(&mut node.children, rest);
    }
}

impl eframe::App for CadViewApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "recent_files", &self.recent);
        eframe::set_value(storage, "dark_theme", &self.dark_theme);
        eframe::set_value(storage, "mouse_rotate", &self.mouse_rotate);
        eframe::set_value(storage, "mouse_pan", &self.mouse_pan);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        match self.dark_theme {
            Some(true) => ui.ctx().set_theme(egui::ThemePreference::Dark),
            Some(false) => ui.ctx().set_theme(egui::ThemePreference::Light),
            None => ui.ctx().set_theme(egui::ThemePreference::System),
        }
        self.poll_load();
        self.poll_measure();
        self.handle_drop(ui);
        // Esc 取消激活的工具;Enter 在坐标系第二步用世界 Z 完成。
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) && self.tool.take().is_some() {
            self.status = "工具已取消".into();
            self.refresh_highlight();
        }
        if ui.input(|i| i.key_pressed(egui::Key::Enter))
            && let Some(Tool::CCsys(Some(o))) = self.tool.clone()
        {
            self.finish_csys(o, Vec3::Z);
        }
        // 选中对象的质量属性/面信息懒加载(后台)。
        let lazy = self.sel.as_ref().map(|s| (s.path.clone(), s.face));
        if let Some((path, face)) = lazy {
            match face {
                Some(f) => self.request_face(path, f),
                None => self.request_mass(path),
            }
        }
        self.draw_toolbar(ui);
        self.draw_tree(ui);
        self.draw_info(ui);
        self.draw_status(ui);
        if self.scene_dirty {
            self.refresh_scene();
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let (rect, response) =
                    ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
                self.handle_input(&response);
                if response.clicked() {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let (x, y) = (pos.x - rect.left(), pos.y - rect.top());
                        let hit = self.pick_hit(x, y);
                        if !self.tool_click(hit.as_ref(), x, y) {
                            self.set_sel(hit.map(|h| Sel { path: h.path, face: Some(h.face) }));
                        }
                    }
                } else if response.double_clicked() && self.tool.is_none() {
                    // 双击:从面升级为整体选中。
                    if let Some(pos) = response.interact_pointer_pos()
                        && let Some(hit) =
                            self.pick_hit(pos.x - rect.left(), pos.y - rect.top())
                    {
                        self.set_sel(Some(Sel { path: hit.path, face: None }));
                    }
                }
                self.vp.show(ui, rect);
                self.draw_axes_gizmo(ui, rect);
                // 叠加标签(测量结果/质心/构造)投影到屏幕。
                for (mid, text, color) in self.overlay_labels() {
                    if let Some((sx, sy, _)) = self.vp.cam.to_screen(
                        mid,
                        f64::from(self.vp.size[0]),
                        f64::from(self.vp.size[1]),
                        self.vp.radius,
                    ) {
                        ui.painter().text(
                            egui::pos2(sx as f32, sy as f32),
                            egui::Align2::CENTER_TOP,
                            text,
                            egui::FontId::monospace(13.0),
                            color,
                        );
                    }
                }
                if let Some(name) = self.loading.as_ref() {
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        format!("正在导入 {name} …"),
                        egui::FontId::proportional(16.0),
                        Color32::from_gray(180),
                    );
                } else if self.doc.is_none() {
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "把 STEP/IGES 文件拖进来",
                        egui::FontId::proportional(18.0),
                        Color32::from_gray(120),
                    );
                }
            });
    }
}

impl Default for CadViewApp {
    fn default() -> Self {
        Self::with_viewport(Viewport::new(), Vec::new(), Some(true), 0, 2)
    }
}
