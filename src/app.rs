//! 主窗口:照 SolidCraft 的查看器布局——自绘标题栏应用栏(品牌 + 文件菜单 + 窗控按钮)、
//! 页签式 ribbon(视图 / 检查)、左侧模型树与右侧属性为浮动圆角卡片、状态走视口内
//! 提示(工具提示在视口左上角,一次性消息为底部 toast,无底部状态栏)。

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, channel};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use egui::{Align2, Color32, FontId, Order, Pos2, Rect, Sense, Stroke, pos2, vec2};
use solidcraft_geom::Vec3;

use crate::fonts;
use crate::scene::{Doc, Node, build_scene, load};
use crate::theme::{self, Tokens};
use crate::titlebar;
use crate::viewport::Viewport;

// ---------------- 布局常量(照 SolidCraft toolbar.rs) ----------------

/// 应用栏(自绘标题栏)高度。
const APP_BAR_HEIGHT: f32 = 34.0;
/// ribbon 页签行高。
const TAB_ROW_HEIGHT: f32 = 30.0;
/// ribbon 按钮组行高(带标签的大按钮模式)。
const GROUP_HEIGHT: f32 = 86.0;
/// 图标 + 文字大按钮高。
const BUTTON_HEIGHT: f32 = 60.0;
/// ribbon 总高:页签行 + 按钮行 + 1px 分隔。
const RIBBON_HEIGHT: f32 = TAB_ROW_HEIGHT + GROUP_HEIGHT + 1.0;
/// 按钮间距 / 最小最大宽。
const BUTTON_GAP: f32 = 3.0;
const BUTTON_MIN_WIDTH: f32 = 54.0;
const BUTTON_MAX_WIDTH: f32 = 76.0;
/// ribbon 按钮图标尺寸。
const RIBBON_ICON: f32 = 26.0;
/// 左侧模型树卡片宽。
const BROWSER_WIDTH: f32 = 250.0;
/// 右侧属性卡片宽(照 SolidCraft form_card_width)。
const PROPS_WIDTH: f32 = 264.0;
/// 树行高(照 SolidCraft TREE_ROW_HEIGHT)。
const TREE_ROW_HEIGHT: f32 = 26.0;
/// 导航罗盘保留区边长。
const COMPASS_SIZE: f32 = 116.0;
/// 罗盘轴长(照 SolidCraft AXIS_LENGTH × AXIS_ART_SCALE)。
const AXIS_LEN: f64 = 38.0 * 0.84;
/// 轴尖命中半径(照 SolidCraft AXIS_RADIUS)。
const AXIS_HIT: f32 = 15.0;
/// toast 存活时长。
const TOAST_LIFE: Duration = Duration::from_secs(6);
/// toast 最多同时显示条数。
const TOAST_MAX: usize = 3;
/// 文件菜单宽。
const FILE_MENU_WIDTH: f32 = 220.0;

/// 当前选中:节点路径 + 可选的 B-rep 面号(None = 整体选中)。
#[derive(Clone, PartialEq)]
struct Sel {
    path: Vec<usize>,
    face: Option<u32>,
}

/// 几何特征列表的选中项(与模型树的 Sel 互斥)。
#[derive(Clone, Copy, PartialEq)]
enum FeatSel {
    /// 点距结果。
    P2p,
    /// 体距结果。
    B2b,
    /// 构造元素(下标进 constructs)。
    Construct(usize),
}

/// 选中反馈配色。
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
}

/// 坐标系对话框正在视口拾取的字段。
#[derive(Clone, Copy, PartialEq)]
enum CsysField {
    Origin,
    ZDir,
}

/// 坐标系「定义/修改」对话框(照 CATIA:默认关联拾取的几何,可去关联成纯数字)。
#[derive(Clone)]
struct CsysDlg {
    /// 编辑现有元素(Some = constructs 下标);None = 新建。
    editing: Option<usize>,
    /// 正在视口拾取的字段;None = 不在拾取。
    armed: Option<CsysField>,
    /// 关联引用(原点/Z 方向)。
    origin: Option<crate::construct::OriginRef>,
    z: Option<crate::construct::DirRef>,
    /// 关联开关(默认开);去关联后用下面两组数字。
    attached: bool,
    /// 独立模式的数字(原点/Z 方向)。
    free_origin: Vec3,
    free_z: Vec3,
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
/// 坐标系三轴(右手法则;与罗盘三轴同色系)。
const CSYS_X: [u8; 4] = [214, 93, 93, 255];
const CSYS_Y: [u8; 4] = [120, 200, 110, 255];
const CSYS_Z: [u8; 4] = [110, 150, 235, 255];
/// 几何特征列表选中项的高亮色(强调色,照 Tokens.accent 暗值)。
const FEAT_SEL_COLOR: [u8; 4] = [58, 136, 238, 255];

/// 图标资源目录(编译期定位到项目 assets/,与运行目录无关)。
const ICON_DIR: &str = env!("CARGO_MANIFEST_DIR");

/// 图标 SVG 里 `currentColor` 注入的颜色:暗主题用亮墨,亮主题用深墨(随 Tokens.text)。
fn icon_ink(dark: bool) -> &'static str {
    if dark {
        "#dee2e8"
    } else {
        "#262a32"
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

/// ribbon 页签。
#[derive(Clone, Copy, PartialEq)]
enum Tab {
    View,
    Inspect,
}

/// 底部 toast:一次性消息(照 SolidCraft 的视口 toast)。
struct Toast {
    text: String,
    error: bool,
    born: Instant,
}

impl Toast {
    fn alive(&self) -> bool {
        self.born.elapsed() < TOAST_LIFE
    }
}

/// ribbon 按钮动作(绘制时收集,绘制完统一应用,避免闭包借用冲突)。
#[derive(Clone, Copy, PartialEq)]
enum Action {
    Fit,
    Front,
    Iso,
    Props,
    Centroid,
    Section,
    P2P,
    B2B,
    CPoint,
    CAxis2P,
    CAxisEdge,
    CAxisFace,
    CCsys,
}

pub struct CadViewApp {
    vp: Viewport,
    doc: Option<Doc>,
    scene_dirty: bool,
    sel: Option<Sel>,
    /// 几何特征列表的选中(与 sel 互斥)。
    feat_sel: Option<FeatSel>,
    section: Option<Section>,
    tool: Option<Tool>,
    /// 工具进行中的常驻提示(视口左上角显示)。
    tool_prompt: String,
    /// 点-点测量结果。
    p2p: Option<(Vec3, Vec3)>,
    /// 点距结果是否显示(几何特征列表的眼睛)。
    show_p2p: bool,
    /// 体-体测量结果(后台完成前为 pending)。
    b2b: Option<B2BEntry>,
    /// 体距结果是否显示(几何特征列表的眼睛)。
    show_b2b: bool,
    /// 体质量属性缓存(选中时懒加载)。
    mass: HashMap<Vec<usize>, Option<MassData>>,
    /// 面信息缓存。
    face_info: HashMap<(Vec<usize>, u32), Option<FaceData>>,
    mrx: Option<Receiver<MeasureMsg>>,
    /// 最近打开的文件(新→旧,持久化)。
    recent: Vec<String>,
    /// 加载完成后下一次 refresh_scene 是否重新适配视图。
    fit_pending: bool,
    /// 右侧属性卡片是否显示。
    show_props: bool,
    /// 是否在选中体上显示质心标记。
    show_centroid: bool,
    /// 设置浮窗是否打开。
    show_settings: bool,
    /// 断面浮窗是否打开(浮窗开着 = 断面生效)。
    show_section_dialog: bool,
    /// 亮/暗主题(None = 跟随系统)。
    dark_theme: Option<bool>,
    /// 上一帧的主题(变了就重建场景,换视口背景/网格色)。
    prev_dark: Option<bool>,
    /// 当前 ribbon 页签。
    tab: Tab,
    /// 文件菜单是否展开。
    file_menu_open: bool,
    /// 文件按钮矩形(菜单锚点;应用栏每帧写入)。
    file_menu_rect: Option<Rect>,
    /// 视口底部 toast 队列(新在后)。
    toasts: Vec<Toast>,
    /// 旋转/平移的鼠标键(持久化)。
    mouse_rotate: u8,
    mouse_pan: u8,
    rx: Option<Receiver<LoadMsg>>,
    loading: Option<String>,
    /// 构造元素(点/轴/坐标系),换文件时清空。
    constructs: Vec<crate::construct::Construct>,
    /// 坐标系「定义/修改」对话框;开着 = 生效(新建中或编辑中)。
    csys_dlg: Option<CsysDlg>,
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
            feat_sel: None,
            section: None,
            tool: None,
            tool_prompt: String::new(),
            p2p: None,
            show_p2p: true,
            b2b: None,
            show_b2b: true,
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
            prev_dark: None,
            tab: Tab::Inspect,
            file_menu_open: false,
            file_menu_rect: None,
            toasts: Vec::new(),
            mouse_rotate,
            mouse_pan,
            rx: None,
            loading: None,
            constructs: Vec::new(),
            csys_dlg: None,
        }
    }

    /// 启动参数里带的文件(创建后调用一次)。
    pub fn open_on_start(&mut self, path: &str) {
        self.start_load(path.to_string());
    }

    /// 弹出一条一次性消息(视口底部 toast)。
    fn notify(&mut self, text: impl Into<String>) {
        self.toasts.push(Toast { text: text.into(), error: false, born: Instant::now() });
        let excess = self.toasts.len().saturating_sub(TOAST_MAX);
        self.toasts.drain(0..excess);
    }

    /// 弹出一条错误消息(红点 toast)。
    fn notify_error(&mut self, text: impl Into<String>) {
        self.toasts.push(Toast { text: text.into(), error: true, born: Instant::now() });
        let excess = self.toasts.len().saturating_sub(TOAST_MAX);
        self.toasts.drain(0..excess);
    }

    /// 结束当前工具(清提示,刷新高亮)。
    fn end_tool(&mut self) {
        self.tool = None;
        self.tool_prompt.clear();
        self.refresh_highlight();
    }

    /// 后台线程加载;UI 每帧只收消息。
    fn start_load(&mut self, path: String) {
        let (tx, rx) = channel();
        self.rx = Some(rx);
        self.loading = Some(path.clone());
        self.file_menu_open = false;
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
                        self.notify(format!(
                            "{}  ·  {}  ·  {} 体 / {} 三角形 / 导入 {} ms",
                            path,
                            doc.schema.split_whitespace().next().unwrap_or("STEP"),
                            doc.bodies(),
                            doc.triangles(),
                            doc.load_ms
                        ));
                        for w in &doc.warnings {
                            log::warn!("导入警告: {w}");
                        }
                        self.doc = Some(doc);
                        self.sel = None;
                        self.feat_sel = None;
                        self.end_tool();
                        self.p2p = None;
                        self.show_p2p = true;
                        self.b2b = None;
                        self.show_b2b = true;
                        self.mass.clear();
                        self.face_info.clear();
                        self.constructs.clear();
                        self.csys_dlg = None; // 对话框引用的元素随文件清空
                        self.push_recent(path.clone());
                        self.fit_pending = true;
                        self.scene_dirty = true;
                    }
                    Err(e) => {
                        self.notify_error(format!("打开失败:{path} — {e}"));
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

    /// 场景内容变化(加载完成/显隐切换/主题切换)时重建渲染场景;视图只在加载后重适配。
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

    /// 弹出导入文件对话框。过滤器照 SolidCraft 的 pick_open:一个全格式组打头,
    /// 再每个格式一组;扩展名大小写各给一份(Linux 对话框按大小写匹配)。
    /// DXF/SVG 草稿格式没有对应的草图功能,不收。
    fn open_dialog(&mut self) {
        // (组名, 扩展名)——与 scene.rs 的 load 分派一一对应。
        const GROUPS: [(&str, &[&str]); 9] = [
            ("STEP", &["step", "stp"]),
            ("IGES", &["igs", "iges"]),
            ("FreeCAD", &["fcstd"]),
            ("Rhino", &["3dm"]),
            ("CATIA", &["catpart"]),
            ("STL", &["stl"]),
            ("3MF", &["3mf"]),
            ("OBJ", &["obj"]),
            ("glTF", &["glb", "gltf"]),
        ];
        let case_variants = |exts: &[&str]| -> Vec<String> {
            let mut all = Vec::new();
            for e in exts {
                for v in [e.to_ascii_lowercase(), e.to_ascii_uppercase()] {
                    if !all.contains(&v) {
                        all.push(v);
                    }
                }
            }
            all
        };
        let combined = case_variants(&GROUPS.iter().flat_map(|(_, e)| e.iter().copied()).collect::<Vec<_>>());
        let per_group: Vec<Vec<String>> = GROUPS.iter().map(|(_, e)| case_variants(e)).collect();
        let mut dlg = rfd::FileDialog::new();
        let combined: Vec<&str> = combined.iter().map(String::as_str).collect();
        dlg = dlg.add_filter("CAD 和网格文件", &combined);
        for ((name, _), exts) in GROUPS.iter().zip(&per_group) {
            let exts: Vec<&str> = exts.iter().map(String::as_str).collect();
            dlg = dlg.add_filter(*name, &exts);
        }
        if let Some(path) = dlg.add_filter("所有文件", &["*"]).pick_file() {
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
                match &r {
                    Some((d, _, _, exact)) => {
                        let approx = if *exact { "" } else { " ≈" };
                        self.notify(format!("体距:{d:.3} mm{approx}"));
                    }
                    None => self.notify_error("体-体测距:两体不相交且超出可测范围,无结果"),
                }
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
        self.show_b2b = true;
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

    /// 构造轴的半长:随场景尺度(包围球半径的 0.35,照 SolidCraft)。
    fn axis_len(&self) -> f64 {
        self.vp.radius.max(1.0) * 0.35
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
                        self.show_p2p = true;
                        let d = (b - a).len();
                        self.end_tool();
                        self.notify(format!("点距:{d:.3} mm"));
                    } else {
                        self.tool_prompt = "点-点测距:再点一个点".into();
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
                        self.end_tool();
                        self.request_body_dist(a, b);
                        self.notify("体-体测距:计算中…");
                    } else {
                        self.tool_prompt = "体-体测距:再点一个体".into();
                        self.tool = Some(Tool::B2B(bodies));
                    }
                    self.refresh_highlight();
                }
                true
            }
            Tool::CPoint => {
                if let Some(p) = self.snap_point(x, y, hit) {
                    let name = crate::construct::next_name(&self.constructs, "P");
                    self.notify(format!("{name}:({:.2}, {:.2}, {:.2})", p.x, p.y, p.z));
                    self.constructs.push(crate::construct::Construct::Point { name, p, visible: true });
                    self.refresh_highlight();
                } else {
                    self.notify("没点到模型;点顶点/圆弧/面上");
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
                                self.constructs.push(crate::construct::Construct::Axis {
                                    name: name.clone(),
                                    origin: a,
                                    dir,
                                    len: self.axis_len(),
                                    visible: true,
                                });
                                self.end_tool();
                                self.notify(format!("{name}:两点轴"));
                            }
                            None => self.notify_error("两点重合,轴无法建立"),
                        }
                    } else {
                        self.tool_prompt = "两点轴:再点一个点".into();
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
                                self.constructs.push(crate::construct::Construct::Axis { name: name.clone(), origin, dir, len: self.axis_len(), visible: true });
                                self.end_tool();
                                self.notify(format!("{name}:{how}轴"));
                            }
                            None => self.notify("这条边不是直线也不是圆弧"),
                        }
                    } else {
                        self.notify("没点到边;点模型的棱线");
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
                            self.constructs.push(crate::construct::Construct::Axis { name: name.clone(), origin, dir, len: self.axis_len(), visible: true });
                            self.end_tool();
                            self.notify(format!("{name}:圆柱/圆锥/圆环轴"));
                        }
                        None => self.notify("这个面不是圆柱/圆锥/圆环面"),
                    }
                } else {
                    self.notify("没点到模型;点一个圆类面");
                }
                true
            }
        }
    }

    // ---------------- 坐标系对话框:视口拾取与提交 ----------------

    /// 屏幕上最近的可见构造点 P{n}(像素距离 ≤ tol)。
    fn nearest_construct_point(&self, x: f32, y: f32, tol: f64) -> Option<(String, Vec3)> {
        let (w, h) = (f64::from(self.vp.size[0]), f64::from(self.vp.size[1]));
        let mut best: Option<(f64, String, Vec3)> = None;
        for c in &self.constructs {
            if let crate::construct::Construct::Point { name, p, visible } = c
                && *visible
                && let Some((sx, sy, _)) = self.vp.cam.to_screen(*p, w, h, self.vp.radius)
            {
                let d = ((sx - f64::from(x)).powi(2) + (sy - f64::from(y)).powi(2)).sqrt();
                if d <= tol && best.as_ref().is_none_or(|(bd, ..)| d < *bd) {
                    best = Some((d, name.clone(), *p));
                }
            }
        }
        best.map(|(_, name, p)| (name, p))
    }

    /// 屏幕上最近的可见构造轴 A{n}(到轴线段的像素距离 ≤ tol)。
    fn nearest_construct_axis(&self, x: f32, y: f32, tol: f64) -> Option<(String, Vec3)> {
        let (w, h) = (f64::from(self.vp.size[0]), f64::from(self.vp.size[1]));
        let mut best: Option<(f64, String, Vec3)> = None;
        for c in &self.constructs {
            if let crate::construct::Construct::Axis { name, origin, dir, len, visible } = c
                && *visible
            {
                let (a, b) = (*origin - *dir * *len, *origin + *dir * *len);
                let Some(d) = crate::pick::screen_dist_to_seg(&self.vp.cam, f64::from(x), f64::from(y), w, h, self.vp.radius, a, b) else {
                    continue;
                };
                if d <= tol && best.as_ref().is_none_or(|(bd, ..)| d < *bd) {
                    best = Some((d, name.clone(), *dir));
                }
            }
        }
        best.map(|(_, name, dir)| (name, dir))
    }

    /// 吸附拾取并生成原点引用(构造点 → 顶点 → 圆弧圆心 → 面中心 → 面上点)。
    fn pick_origin_ref(&self, x: f32, y: f32, hit: Option<&crate::pick::Hit>) -> Option<crate::construct::OriginRef> {
        let doc = self.doc.as_ref()?;
        let (w, h) = (f64::from(self.vp.size[0]), f64::from(self.vp.size[1]));
        if let Some((name, _)) = self.nearest_construct_point(x, y, 9.0) {
            return Some(crate::construct::OriginRef::ConstructPoint { name });
        }
        if let Some(v) = crate::pick::pick_vertex(doc, &self.vp.cam, f64::from(x), f64::from(y), w, h, self.vp.radius, 9.0) {
            return Some(crate::construct::OriginRef::Vertex { path: v.path, at: v.point });
        }
        if let Some(e) = crate::pick::pick_edge(doc, &self.vp.cam, f64::from(x), f64::from(y), w, h, self.vp.radius, 6.0)
            && let Some((center, _)) = crate::construct::circle_edge(doc, &e.path, e.edge)
        {
            return Some(crate::construct::OriginRef::ArcCenter { path: e.path, edge: e.edge, at: center });
        }
        if let Some(h) = hit {
            if let Some(c) = crate::construct::round_face_center(doc, &h.path, h.face) {
                return Some(crate::construct::OriginRef::FaceCenter { path: h.path.clone(), face: h.face, at: c });
            }
            return Some(crate::construct::OriginRef::SurfacePoint { path: h.path.clone(), face: h.face, at: h.point });
        }
        None
    }

    /// 吸附拾取并生成 Z 方向引用(构造轴 → 直边方向 / 圆弧轴向 / 圆柱类面轴向)。
    fn pick_dir_ref(&self, x: f32, y: f32, hit: Option<&crate::pick::Hit>) -> Option<crate::construct::DirRef> {
        let doc = self.doc.as_ref()?;
        let (w, h) = (f64::from(self.vp.size[0]), f64::from(self.vp.size[1]));
        if let Some((name, _)) = self.nearest_construct_axis(x, y, 9.0) {
            return Some(crate::construct::DirRef::ConstructAxis { name });
        }
        if let Some(e) = crate::pick::pick_edge(doc, &self.vp.cam, f64::from(x), f64::from(y), w, h, self.vp.radius, 6.0) {
            if crate::construct::straight_edge(&e.polyline).is_some() {
                return Some(crate::construct::DirRef::Edge { path: e.path, edge: e.edge });
            }
            if crate::construct::circle_edge(doc, &e.path, e.edge).is_some() {
                return Some(crate::construct::DirRef::ArcAxis { path: e.path, edge: e.edge });
            }
        }
        if let Some(h) = hit
            && crate::construct::round_face_axis(doc, &h.path, h.face).is_some()
        {
            return Some(crate::construct::DirRef::FaceAxis { path: h.path.clone(), face: h.face });
        }
        None
    }

    /// 坐标系对话框拾取中:吃掉一次视口点击,填入激活字段并自动跳下一个。
    /// 返回 true = 点击已消费。
    fn csys_pick(&mut self, hit: Option<&crate::pick::Hit>, x: f32, y: f32) -> bool {
        let Some(field) = self.csys_dlg.as_ref().and_then(|d| d.armed) else { return false };
        match field {
            CsysField::Origin => match self.pick_origin_ref(x, y, hit) {
                Some(r) => {
                    let dlg = self.csys_dlg.as_mut().expect("armed 才会进来");
                    dlg.origin = Some(r);
                    dlg.armed = if dlg.z.is_none() { Some(CsysField::ZDir) } else { None };
                }
                None => self.notify("没点到模型;点顶点/圆弧圆心/圆类面中心/面上"),
            },
            CsysField::ZDir => match self.pick_dir_ref(x, y, hit) {
                Some(r) => {
                    let dlg = self.csys_dlg.as_mut().expect("armed 才会进来");
                    dlg.z = Some(r);
                    dlg.armed = None;
                }
                None => self.notify("没拾取到方向;点直边/圆弧边/圆柱类面"),
            },
        }
        self.refresh_highlight();
        true
    }

    /// 对话框 [确定]/Enter:校验并新建或原地更新;失败保留对话框。
    fn commit_csys(&mut self) {
        let Some(dlg) = self.csys_dlg.clone() else { return };
        let Some((o, x, y, z)) = csys_frame(
            self.doc.as_ref(),
            &self.constructs,
            dlg.attached,
            dlg.origin.as_ref(),
            dlg.z.as_ref(),
            dlg.free_origin,
            dlg.free_z,
        ) else {
            self.notify_error("原点或 Z 方向未定/失效,建不了坐标系");
            return;
        };
        let def = if dlg.attached {
            Some(crate::construct::CsysDef { origin: dlg.origin.clone().expect("attached 时 frame 已校验"), z: dlg.z.clone().expect("attached 时 frame 已校验") })
        } else {
            None
        };
        match dlg.editing {
            None => {
                let name = crate::construct::next_name(&self.constructs, "C");
                self.notify(format!("{name}:坐标系({})", if dlg.attached { "关联" } else { "独立" }));
                self.constructs.push(crate::construct::Construct::Csys { name, origin: o, x, y, z, def, visible: true });
            }
            Some(i) => {
                if let Some(crate::construct::Construct::Csys { name, visible, .. }) = self.constructs.get(i) {
                    let (name, visible) = (name.clone(), *visible);
                    self.constructs[i] = crate::construct::Construct::Csys { name: name.clone(), origin: o, x, y, z, def, visible };
                    self.notify(format!("{name}:坐标系已更新"));
                }
            }
        }
        self.close_csys_dialog();
    }

    /// 打开编辑现有坐标系的对话框(关联的预填引用,独立的预填数字)。
    fn open_csys_editor(&mut self, i: usize) {
        let Some(crate::construct::Construct::Csys { origin, x: _, y: _, z, def, .. }) = self.constructs.get(i) else {
            return;
        };
        let (ro, rz, attached) = match def {
            Some(d) => (Some(d.origin.clone()), Some(d.z.clone()), true),
            None => (None, None, false),
        };
        self.csys_dlg = Some(CsysDlg {
            editing: Some(i),
            armed: None,
            origin: ro,
            z: rz,
            attached,
            free_origin: *origin,
            free_z: *z,
        });
        self.refresh_highlight();
    }

    /// 关闭坐标系对话框(丢弃未提交的修改)。
    fn close_csys_dialog(&mut self) {
        self.csys_dlg = None;
        self.refresh_highlight();
    }

    fn set_sel(&mut self, sel: Option<Sel>) {
        self.sel = sel;
        self.feat_sel = None;
        self.refresh_highlight();
    }

    /// 切换几何特征选中(与模型树选中互斥)。
    fn toggle_feat_sel(&mut self, f: FeatSel) {
        self.feat_sel = if self.feat_sel == Some(f) { None } else { Some(f) };
        if self.feat_sel.is_some() {
            self.sel = None;
        }
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

    fn highlight_scene(&mut self) -> Option<solidcraft_gpu::view::GpuScene> {
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
        // 关联坐标系先按引用重解析(几何变了轴系跟着走;引用失效则保留上次数字)。
        {
            let mut upd: Vec<(usize, Vec3, Vec3, Vec3, Vec3)> = Vec::new();
            for (i, c) in self.constructs.iter().enumerate() {
                if let crate::construct::Construct::Csys { def: Some(def), .. } = c
                    && let Some(frame) = crate::construct::resolve_csys(doc, &self.constructs, def)
                {
                    upd.push((i, frame.0, frame.1, frame.2, frame.3));
                }
            }
            for (i, o, x, y, z) in upd {
                if let Some(crate::construct::Construct::Csys { origin, x: xx, y: yy, z: zz, .. }) = self.constructs.get_mut(i) {
                    *origin = o;
                    *xx = x;
                    *yy = y;
                    *zz = z;
                }
            }
        }
        // 测量/构造结果按几何特征列表的眼睛显隐;列表选中的用强调色。
        let p2p_col = if self.feat_sel == Some(FeatSel::P2p) { FEAT_SEL_COLOR } else { MEASURE_COLOR };
        if self.show_p2p
            && let Some((a, b)) = &self.p2p
        {
            mark(&mut sc, *a, p2p_col);
            mark(&mut sc, *b, p2p_col);
            sc.line(a.to_f32(), b.to_f32(), p2p_col, 2.0, true);
        }
        let b2b_col = if self.feat_sel == Some(FeatSel::B2b) { FEAT_SEL_COLOR } else { MEASURE_COLOR };
        if self.show_b2b
            && let Some((_, Some((_, from, to, _)))) = &self.b2b
        {
            mark(&mut sc, *from, b2b_col);
            mark(&mut sc, *to, b2b_col);
            sc.line(from.to_f32(), to.to_f32(), b2b_col, 2.0, true);
        }
        // 质心标记(开启且选中体的质量属性已算出)。
        if self.show_centroid
            && let Some(sel) = &self.sel
            && sel.face.is_none()
            && let Some(Some((_, _, c, _))) = self.mass.get(&sel.path)
        {
            mark(&mut sc, *c, CENTROID_COLOR);
        }
        // 构造元素:点十字 / 轴线 / 三色坐标系(画在最上层;眼睛隐藏的跳过)。
        let arm = self.axis_len();
        for (i, c) in self.constructs.iter().enumerate() {
            if !c.visible() {
                continue;
            }
            let sel_c = self.feat_sel == Some(FeatSel::Construct(i));
            let point_col = if sel_c { FEAT_SEL_COLOR } else { CONSTRUCT_POINT };
            let axis_col = if sel_c { FEAT_SEL_COLOR } else { CONSTRUCT_AXIS };
            match c {
                crate::construct::Construct::Point { p, .. } => mark(&mut sc, *p, point_col),
                crate::construct::Construct::Axis { origin, dir, len, .. } => {
                    let (a, b) = (*origin - *dir * *len, *origin + *dir * *len);
                    sc.line(a.to_f32(), b.to_f32(), axis_col, 2.0, true);
                    mark(&mut sc, *origin, axis_col);
                }
                crate::construct::Construct::Csys { origin, x, y, z, .. } => {
                    for (d, col) in [(*x, CSYS_X), (*y, CSYS_Y), (*z, CSYS_Z)] {
                        let col = if sel_c { FEAT_SEL_COLOR } else { col };
                        let e = *origin + d * arm;
                        sc.line(origin.to_f32(), e.to_f32(), col, 3.5, true);
                    }
                    mark(&mut sc, *origin, point_col);
                }
            }
        }
        // 坐标系对话框的实时预览:当前引用/数字的解算结果(照 SolidCraft 的预览)。
        if let Some(dlg) = &self.csys_dlg
            && let Some((o, x, y, z)) =
                csys_frame(Some(doc), &self.constructs, dlg.attached, dlg.origin.as_ref(), dlg.z.as_ref(), dlg.free_origin, dlg.free_z)
        {
            for (d, col) in [(x, CSYS_X), (y, CSYS_Y), (z, CSYS_Z)] {
                let e = o + d * arm;
                sc.line(o.to_f32(), e.to_f32(), col, 3.5, true);
            }
            mark(&mut sc, o, CONSTRUCT_POINT);
        }
        Some(sc)
    }

    /// 屏幕叠加标签(测量结果/质心/构造元素),返回 (世界点, 文本, 颜色, 屏幕偏移像素)。
    fn overlay_labels(&self) -> Vec<(Vec3, String, Color32, [f32; 2])> {
        let measure = Color32::from_rgb(120, 230, 140);
        let construct = Color32::from_rgb(255, 190, 90);
        let feat_sel = Color32::from_rgb(FEAT_SEL_COLOR[0], FEAT_SEL_COLOR[1], FEAT_SEL_COLOR[2]);
        let mut out = Vec::new();
        if self.show_p2p
            && let Some((a, b)) = &self.p2p
        {
            let col = if self.feat_sel == Some(FeatSel::P2p) { feat_sel } else { measure };
            out.push(((*a + *b) * 0.5, format!("{:.3} mm", (*b - *a).len()), col, [0.0, 10.0]));
        }
        if self.show_b2b
            && let Some((_, Some((d, from, to, exact)))) = &self.b2b
        {
            let col = if self.feat_sel == Some(FeatSel::B2b) { feat_sel } else { measure };
            let approx = if *exact { "" } else { " ≈" };
            out.push(((*from + *to) * 0.5, format!("{d:.3} mm{approx}"), col, [0.0, 10.0]));
        }
        if self.show_centroid
            && let Some(sel) = &self.sel
            && sel.face.is_none()
            && let Some(Some((_, _, c, _))) = self.mass.get(&sel.path)
        {
            out.push((*c, format!("质心 ({:.1}, {:.1}, {:.1})", c.x, c.y, c.z), measure, [0.0, -10.0]));
        }
        use crate::construct::Construct;
        for (i, c) in self.constructs.iter().enumerate() {
            if !c.visible() {
                continue;
            }
            let col = if self.feat_sel == Some(FeatSel::Construct(i)) { feat_sel } else { construct };
            match c {
                Construct::Point { name, p, .. } => {
                    out.push((*p, format!("{name} ({:.1}, {:.1}, {:.1})", p.x, p.y, p.z), col, [0.0, -12.0]));
                }
                Construct::Axis { name, origin, dir, len, .. } => {
                    out.push((*origin + *dir * *len, name.clone(), col, [0.0, -12.0]));
                }
                // 坐标系的名字放在原点上方,避开三根轴线。
                Construct::Csys { name, origin, .. } => out.push((*origin, name.clone(), col, [0.0, -14.0])),
            }
        }
        // 坐标系的 X/Y/Z 轴端字母(贴轴端外侧,同罗盘配色;隐藏的坐标系不标)。
        let arm = self.axis_len();
        for c in &self.constructs {
            if !c.visible() {
                continue;
            }
            if let Construct::Csys { origin, x, y, z, .. } = c {
                for (d, label, col) in [
                    (*x, "X", Color32::from_rgb(224, 106, 106)),
                    (*y, "Y", Color32::from_rgb(130, 210, 120)),
                    (*z, "Z", Color32::from_rgb(120, 160, 245)),
                ] {
                    out.push((*origin + d * arm * 1.15, label.into(), col, [0.0, 0.0]));
                }
            }
        }
        out
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

    // ---------------- 应用栏(自绘标题栏) ----------------

    /// 34px 应用栏:品牌标 + CADViewCraft + 文件菜单 + 设置齿轮 + 版本号 + 窗控按钮。
    /// 兼当标题栏:空白处拖动窗口、双击最大化。
    fn draw_app_bar(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get();
        let dark = theme::is_dark();
        egui::Panel::top("app_bar")
            .exact_size(APP_BAR_HEIGHT)
            .show_separator_line(false)
            .frame(egui::Frame::default().fill(t.app_bar).inner_margin(egui::Margin::same(0)))
            .show(ui, |ui| {
                let r = ui.max_rect();
                // 空白处拖动窗口(要在按钮注册前画,按钮才能拿到自己的点击)。
                titlebar::drag_area(ui, r);
                // 品牌 logo + 名字。
                titlebar::brand_mark(ui.painter(), pos2(r.left() + 20.0, r.center().y), 20.0, t.logo, t.app_bar);
                ui.painter().text(
                    pos2(r.left() + 38.0, r.center().y),
                    Align2::LEFT_CENTER,
                    "CADViewCraft",
                    fonts::semibold(theme::FONT_HEADING),
                    t.app_bar_text,
                );
                let mut x = r.left() + 152.0;
                titlebar::app_bar_separator(ui.painter(), x, r, t.app_bar_separator);
                x += 14.0;
                // 文件菜单:文字按钮(照 SolidCraft app_bar 的 File)。
                let galley = ui.painter().layout_no_wrap("文件".to_owned(), FontId::proportional(theme::FONT_BUTTON), t.app_bar_text);
                let br = Rect::from_min_size(pos2(x, r.center().y - 13.0), vec2(galley.size().x + 16.0, 26.0));
                let resp = ui.interact(br, ui.id().with(("ab", "file")), Sense::click());
                if resp.hovered() || resp.has_focus() || self.file_menu_open {
                    ui.painter().rect_filled(br, theme::RADIUS_SM, t.app_bar_hover);
                }
                ui.painter().galley(pos2(br.left() + 8.0, br.center().y - galley.size().y / 2.0), galley, t.app_bar_text);
                if resp.clicked() {
                    self.file_menu_open = !self.file_menu_open;
                }
                self.file_menu_rect = Some(br);
                // 右簇:设置齿轮 + 版本号,再右是窗控按钮。
                let gear = Rect::from_center_size(pos2(r.right() - titlebar::WIDTH - 44.0, r.center().y), vec2(32.0, 32.0));
                let gresp = ui.interact(gear, ui.id().with(("ab", "settings")), Sense::click());
                if gresp.hovered() || gresp.has_focus() {
                    ui.painter().rect_filled(gear.shrink(3.0), theme::RADIUS_SM, t.app_bar_hover_button);
                }
                ui.put(gear.shrink(7.0), icon_image("settings", 18.0, dark));
                if gresp.clicked() {
                    self.show_settings = !self.show_settings;
                }
                gresp.on_hover_text("设置");
                ui.painter().text(
                    pos2(r.right() - titlebar::WIDTH - 14.0, r.center().y),
                    Align2::RIGHT_CENTER,
                    concat!("v", env!("CARGO_PKG_VERSION")),
                    FontId::proportional(theme::FONT_SMALL),
                    t.app_bar_text.gamma_multiply(0.55),
                );
                titlebar::caption_buttons(ui, r);
            });
    }

    /// 展开中的文件菜单(在浮动卡片之后画,保证在最上层)。返回菜单矩形。
    fn draw_file_menu(&mut self, ui: &mut egui::Ui) {
        let Some(anchor) = self.file_menu_rect.map(|br| br.left_bottom()) else { return };
        let t = Tokens::get();
        let mut close = false;
        let mut menu_rect = Rect::NOTHING;
        egui::Area::new(egui::Id::new("file_menu"))
            .fixed_pos(anchor)
            .order(Order::Foreground)
            .show(ui.ctx(), |ui| {
                egui::Frame::default()
                    .fill(t.panel)
                    .stroke(Stroke::new(1.0, t.border))
                    .corner_radius(egui::CornerRadius::same(theme::RADIUS_MD))
                    .inner_margin(egui::Margin::same(theme::SPACE_4 as i8))
                    .show(ui, |ui| {
                        // Area 的 Ui 不限宽时 justify 会撑成巨块,先钉死菜单宽。
                        ui.set_min_width(FILE_MENU_WIDTH);
                        ui.set_max_width(FILE_MENU_WIDTH);
                        if menu_item(ui, "导入…").clicked() {
                            self.open_dialog();
                            close = true;
                        }
                        if !self.recent.is_empty() {
                            ui.separator();
                            let items = self.recent.clone();
                            for rp in items {
                                let name = std::path::Path::new(&rp)
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| rp.clone());
                                if menu_item(ui, &name).clicked() {
                                    self.start_load(rp);
                                    close = true;
                                }
                            }
                        }
                        ui.separator();
                        if menu_item(ui, "退出").clicked() {
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                menu_rect = ui.min_rect();
            });
        // 点菜单和文件按钮之外的任意处关闭。
        let btn = self.file_menu_rect;
        let clicked_outside = ui.input(|i| i.pointer.any_click())
            && ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| {
                !menu_rect.contains(p) && !btn.is_some_and(|br| br.expand(2.0).contains(p))
            });
        if close || clicked_outside {
            self.file_menu_open = false;
        }
    }

    // ---------------- Ribbon ----------------

    /// 页签式 ribbon:30px 页签行(视图 / 检查)+ 86px 图标按钮行(组间 1px 竖线,无组名)。
    fn draw_ribbon(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get();
        let dark = theme::is_dark();
        egui::Panel::top("ribbon")
            .exact_size(RIBBON_HEIGHT)
            .show_separator_line(false)
            .frame(egui::Frame::default().fill(t.toolbar).inner_margin(egui::Margin::same(0)))
            .show(ui, |ui| {
                let r = ui.max_rect();
                let inset = theme::SPACE_4;
                // --- 页签行 ---
                let mut x = r.left() + inset;
                let mut next_tab = self.tab;
                for (tab, label) in [(Tab::View, "视图"), (Tab::Inspect, "检查")] {
                    let active = self.tab == tab;
                    let color = if active { t.accent_ink } else { t.text };
                    let galley = ui.painter().layout_no_wrap(label.to_owned(), fonts::semibold(theme::FONT_BUTTON), color);
                    let width = galley.size().x + 24.0;
                    let tr = Rect::from_min_size(pos2(x, r.top() + inset), vec2(width, TAB_ROW_HEIGHT - 2.0 * inset));
                    let resp = ui.interact(tr, ui.id().with(("tab", label)), Sense::click());
                    let fill = if active {
                        t.accent_soft
                    } else if resp.hovered() {
                        t.hover
                    } else {
                        Color32::TRANSPARENT
                    };
                    ui.painter().rect_filled(tr, theme::RADIUS_SM, fill);
                    ui.painter().galley(
                        pos2(tr.center().x - galley.size().x / 2.0, tr.center().y - galley.size().y / 2.0),
                        galley,
                        color,
                    );
                    if resp.clicked() {
                        next_tab = tab;
                    }
                    x += width + 2.0;
                }
                if next_tab != self.tab {
                    self.tab = next_tab;
                }
                // 页签行底部分隔线(照 SolidCraft:toolbar 与 border 折半)。
                ui.painter().line_segment(
                    [pos2(r.left(), r.top() + TAB_ROW_HEIGHT - 0.5), pos2(r.right(), r.top() + TAB_ROW_HEIGHT - 0.5)],
                    Stroke::new(1.0, t.secondary_line()),
                );
                // --- 按钮行 ---
                let row_top = r.top() + TAB_ROW_HEIGHT;
                let button_top = row_top + (GROUP_HEIGHT - BUTTON_HEIGHT) / 2.0;
                let mut action: Option<Action> = None;
                let mut cx = r.left() + theme::SPACE_6;
                let groups: &[&[(&str, &str, &str, bool, Action)]] = match self.tab {
                    Tab::View => {
                        &[&[
                            ("fit", "适配", "适配视图:相机对准模型", false, Action::Fit),
                            ("lookat", "前视", "切到前视图", false, Action::Front),
                            ("iso_analysis", "等轴测", "切到等轴测视图", false, Action::Iso),
                            ("appearance", "属性", "开关右侧属性卡片", self.show_props, Action::Props),
                        ]]
                    }
                    Tab::Inspect => {
                        &[
                            &[("cpoint", "建点", "构造点:吸附顶点/圆心,否则面命中点", matches!(self.tool, Some(Tool::CPoint)), Action::CPoint)],
                            &[
                                ("caxis_2p", "两点轴", "过两点的构造轴", matches!(self.tool, Some(Tool::CAxis2P(_))), Action::CAxis2P),
                                ("caxis_edge", "边轴", "直边方向 / 圆弧边圆心与法向", matches!(self.tool, Some(Tool::CAxisEdge)), Action::CAxisEdge),
                                ("caxis_cyl", "圆柱轴", "圆柱/圆锥/圆环面的轴线", matches!(self.tool, Some(Tool::CAxisFace)), Action::CAxisFace),
                            ],
                            &[("ccsys", "坐标系", "坐标系对话框:拾取原点与 Z 方向,默认关联,双击列表行可修改", self.csys_dlg.is_some(), Action::CCsys)],
                            &[
                                ("measure", "点距", "测量两点距离", matches!(self.tool, Some(Tool::P2P(_))), Action::P2P),
                                ("inspect", "体距", "测量两实体最小距离", matches!(self.tool, Some(Tool::B2B(_))), Action::B2B),
                                ("center_of_mass", "质心", "显示选中实体的质心标记", self.show_centroid, Action::Centroid),
                            ],
                            &[("section", "断面", "开关剖切(参数见断面浮窗)", self.section.is_some(), Action::Section)],
                        ]
                    }
                };
                for (gi, group) in groups.iter().enumerate() {
                    if gi > 0 {
                        // 组间 1px 竖分隔线(照 SolidCraft)。
                        ui.painter().line_segment(
                            [pos2(cx, button_top + theme::SPACE_8), pos2(cx, button_top + 46.0)],
                            Stroke::new(1.0, t.secondary_line()),
                        );
                        cx += theme::SPACE_8 + 1.0;
                    }
                    for (icon, label, tip, selected, act) in group.iter() {
                        let galley = ui.painter().layout_no_wrap((*label).to_owned(), FontId::proportional(theme::FONT_SMALL), t.text);
                        let width = (galley.size().x + 16.0).clamp(BUTTON_MIN_WIDTH, BUTTON_MAX_WIDTH);
                        let whole = Rect::from_min_size(pos2(cx, button_top), vec2(width, BUTTON_HEIGHT));
                        let resp = ui.interact(whole, ui.id().with(("rb", gi, *icon)), Sense::click());
                        if *selected {
                            ui.painter().rect(whole, theme::RADIUS_SM, t.accent_soft, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
                        } else if resp.hovered() {
                            ui.painter().rect_filled(whole, theme::RADIUS_SM, t.hover);
                        }
                        let icon_rect = Rect::from_center_size(pos2(whole.center().x, whole.top() + 16.0), vec2(RIBBON_ICON, RIBBON_ICON));
                        let icon_top_left = pos2(icon_rect.left(), icon_rect.top());
                        ui.scope_builder(egui::UiBuilder::new().max_rect(Rect::from_min_size(icon_top_left, vec2(RIBBON_ICON, RIBBON_ICON))), |ui| {
                            ui.add(icon_image(icon, RIBBON_ICON, dark));
                        });
                        ui.painter().galley(
                            pos2(whole.center().x - galley.size().x / 2.0, whole.top() + 32.0),
                            galley,
                            t.text,
                        );
                        if resp.clicked() {
                            action = Some(*act);
                        }
                        resp.on_hover_text(*tip);
                        cx += width + BUTTON_GAP;
                    }
                }
                if let Some(act) = action {
                    self.apply_ribbon(act);
                }
            });
    }

    /// 应用一个 ribbon 按钮动作。
    fn apply_ribbon(&mut self, act: Action) {
        match act {
            Action::Fit => self.fit_view(),
            Action::Front => {
                let (yaw, pitch) = solidcraft_render::StandardView::Front.angles();
                self.vp.cam.yaw = yaw;
                self.vp.cam.pitch = pitch;
            }
            Action::Iso => {
                let (yaw, pitch) = solidcraft_render::StandardView::Iso.angles();
                self.vp.cam.yaw = yaw;
                self.vp.cam.pitch = pitch;
            }
            Action::Props => self.show_props = !self.show_props,
            Action::Centroid => {
                self.show_centroid = !self.show_centroid;
                self.refresh_highlight();
            }
            Action::Section => self.toggle_section(),
            Action::P2P => {
                if matches!(self.tool, Some(Tool::P2P(_))) {
                    self.end_tool();
                    self.notify("测量已取消");
                } else {
                    self.tool = Some(Tool::P2P(Vec::new()));
                    self.p2p = None;
                    self.tool_prompt = "点-点测距:点第一个点(Esc 取消)".into();
                    self.refresh_highlight();
                }
            }
            Action::B2B => {
                if matches!(self.tool, Some(Tool::B2B(_))) {
                    self.end_tool();
                    self.notify("测量已取消");
                } else {
                    self.tool = Some(Tool::B2B(Vec::new()));
                    self.b2b = None;
                    self.tool_prompt = "体-体测距:点第一个体(Esc 取消)".into();
                    self.refresh_highlight();
                }
            }
            Action::CPoint => {
                if self.tool == Some(Tool::CPoint) {
                    self.end_tool();
                    self.notify("工具已取消");
                } else {
                    self.tool = Some(Tool::CPoint);
                    self.tool_prompt = "建点:点顶点/圆弧/圆类面/任意点(Esc 退出)".into();
                    self.refresh_highlight();
                }
            }
            Action::CAxis2P => {
                if matches!(self.tool, Some(Tool::CAxis2P(_))) {
                    self.end_tool();
                    self.notify("工具已取消");
                } else {
                    self.tool = Some(Tool::CAxis2P(Vec::new()));
                    self.tool_prompt = "两点轴:点第一个点(Esc 取消)".into();
                    self.refresh_highlight();
                }
            }
            Action::CAxisEdge => {
                if self.tool == Some(Tool::CAxisEdge) {
                    self.end_tool();
                    self.notify("工具已取消");
                } else {
                    self.tool = Some(Tool::CAxisEdge);
                    self.tool_prompt = "边轴:点一条直边或圆弧边(Esc 取消)".into();
                    self.refresh_highlight();
                }
            }
            Action::CAxisFace => {
                if self.tool == Some(Tool::CAxisFace) {
                    self.end_tool();
                    self.notify("工具已取消");
                } else {
                    self.tool = Some(Tool::CAxisFace);
                    self.tool_prompt = "圆柱轴:点圆柱/圆锥/圆环面(Esc 取消)".into();
                    self.refresh_highlight();
                }
            }
            Action::CCsys => {
                if self.csys_dlg.is_some() {
                    self.close_csys_dialog();
                    self.notify("已取消坐标系");
                } else {
                    self.end_tool(); // 结束其他激活中的工具(建点/测量等)
                    self.csys_dlg = Some(CsysDlg {
                        editing: None,
                        armed: Some(CsysField::Origin),
                        origin: None,
                        z: None,
                        attached: true,
                        free_origin: Vec3::ZERO,
                        free_z: Vec3::Z,
                    });
                    self.refresh_highlight();
                }
            }
        }
    }

    // ---------------- 浮动卡片 ----------------

    /// 视口左上内容区的起始 y(应用栏 + ribbon + 卡片间隙)。
    fn content_top() -> f32 {
        APP_BAR_HEIGHT + RIBBON_HEIGHT + theme::SHELL_GAP
    }

    /// 左侧模型树卡片(浮动,圆角,宽 250)。
    fn draw_browser_card(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get();
        let dark = theme::is_dark();
        let screen = ui.input(|i| i.viewport_rect());
        let top = Self::content_top();
        let rect = Rect::from_min_max(
            pos2(theme::SHELL_GAP, top),
            pos2(theme::SHELL_GAP + BROWSER_WIDTH, screen.bottom() - theme::SHELL_GAP),
        );
        egui::Area::new(egui::Id::new("browser_card"))
            .fixed_pos(rect.min)
            .movable(false)
            .show(ui.ctx(), |ui| {
                ui.set_max_size(vec2(BROWSER_WIDTH, rect.height()));
                crate::theme::card_frame(&t).show(ui, |ui| {
                    // 头部:标题 + 体数(照 SolidCraft browser_header)。
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("模型").heading().color(t.text));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let n = self.doc.as_ref().map(|d| d.bodies()).unwrap_or(0);
                            ui.label(
                                egui::RichText::new(format!("{n} 体"))
                                    .size(theme::FONT_PANEL)
                                    .color(t.text_dim),
                            );
                        });
                    });
                    ui.separator();
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        let Some(doc) = &self.doc else {
                            ui.label(egui::RichText::new("尚未打开文件").color(t.text_dim));
                            ui.label(egui::RichText::new("拖入 STEP/IGES/STL 等,或 应用栏 → 文件 → 导入").color(t.text_dim).size(theme::FONT_SMALL));
                            return;
                        };
                        let mut toggles: Vec<Vec<usize>> = Vec::new();
                        let mut selects: Vec<Vec<usize>> = Vec::new();
                        for (i, root) in doc.roots.iter().enumerate() {
                            tree_node_ui(ui, root, &[i], &mut toggles, &mut selects, self.sel.as_ref(), dark, &t);
                        }
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
                        self.features_ui(ui, dark, &t);
                    });
                });
            });
    }

    /// 左卡「几何特征」小节:测量(点距/体距)+ 构造(点/轴/坐标系)。
    /// 行操作:眼睛显隐、× 删除、点名字选中(视口强调色,与模型树选中互斥)。
    fn features_ui(&mut self, ui: &mut egui::Ui, dark: bool, t: &Tokens) {
        let n_meas = usize::from(self.p2p.is_some()) + usize::from(self.b2b.is_some());
        let n = n_meas + self.constructs.len();
        if n == 0 {
            return;
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(fonts::semibold_text(theme::FONT_BODY, "几何特征").color(t.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(format!("{n} 项")).size(theme::FONT_PANEL).color(t.text_dim));
            });
        });
        // 收集动作,小节画完后统一应用(照模型树 toggles/selects 的模式)。
        let mut eye_p2p = false;
        let mut del_p2p = false;
        let mut sel_p2p = false;
        let mut eye_b2b = false;
        let mut del_b2b = false;
        let mut sel_b2b = false;
        let mut eye_c: Vec<usize> = Vec::new();
        let mut del_c: Vec<usize> = Vec::new();
        let mut sel_c: Vec<usize> = Vec::new();
        let mut edit_c: Vec<usize> = Vec::new();
        if n_meas > 0 {
            egui::CollapsingHeader::new("测量")
                .default_open(true)
                .show(ui, |ui| {
                    if let Some((a, b)) = &self.p2p {
                        let label = format!("点距  {:.3} mm", (*b - *a).len());
                        let (eye, del, name, _) =
                            feature_row_ui(ui, t, dark, "measure", &label, self.show_p2p, self.feat_sel == Some(FeatSel::P2p));
                        (eye_p2p, del_p2p, sel_p2p) = (eye, del, name);
                    }
                    if let Some((_, r)) = &self.b2b {
                        let label = match r {
                            Some((d, _, _, exact)) => format!("体距  {d:.3} mm{}", if *exact { "" } else { " ≈" }),
                            None => "体距  (计算中)".to_string(),
                        };
                        let (eye, del, name, _) =
                            feature_row_ui(ui, t, dark, "measure", &label, self.show_b2b, self.feat_sel == Some(FeatSel::B2b));
                        (eye_b2b, del_b2b, sel_b2b) = (eye, del, name);
                    }
                });
        }
        if !self.constructs.is_empty() {
            egui::CollapsingHeader::new("构造")
                .default_open(true)
                .show(ui, |ui| {
                    for (i, c) in self.constructs.iter().enumerate() {
                        let icon = match c {
                            crate::construct::Construct::Point { .. } => "cpoint",
                            crate::construct::Construct::Axis { .. } => "caxis_2p",
                            crate::construct::Construct::Csys { .. } => "ccsys",
                        };
                        // 坐标系行带关联状态后缀;双击名字进定义/修改对话框。
                        let label = match c {
                            crate::construct::Construct::Csys { def: Some(_), .. } => format!("{} ·关联", c.name()),
                            crate::construct::Construct::Csys { def: None, .. } => format!("{} ·独立", c.name()),
                            _ => c.name().to_string(),
                        };
                        let (eye, del, name, dbl) =
                            feature_row_ui(ui, t, dark, icon, &label, c.visible(), self.feat_sel == Some(FeatSel::Construct(i)));
                        if eye {
                            eye_c.push(i);
                        }
                        if del {
                            del_c.push(i);
                        }
                        if name {
                            sel_c.push(i);
                        }
                        if dbl && matches!(c, crate::construct::Construct::Csys { .. }) {
                            edit_c.push(i);
                        }
                    }
                });
        }
        // 应用动作。
        if eye_p2p {
            self.show_p2p = !self.show_p2p;
        }
        if del_p2p {
            self.p2p = None;
            self.feat_sel = None;
        }
        if sel_p2p {
            self.toggle_feat_sel(FeatSel::P2p);
        }
        if eye_b2b {
            self.show_b2b = !self.show_b2b;
        }
        if del_b2b {
            self.b2b = None;
            self.feat_sel = None;
        }
        if sel_b2b {
            self.toggle_feat_sel(FeatSel::B2b);
        }
        for &i in &eye_c {
            if let Some(c) = self.constructs.get_mut(i) {
                c.set_visible(!c.visible());
            }
        }
        // 从大到小删,避免下标位移;删除同时清特征选中。
        del_c.sort_unstable();
        del_c.dedup();
        // 先记下被删元素的名字:坐标系引用它们时自动去关联。
        let removed: Vec<String> = del_c
            .iter()
            .filter(|&&i| i < self.constructs.len())
            .map(|&i| self.constructs[i].name().to_string())
            .collect();
        for &i in del_c.iter().rev() {
            if i < self.constructs.len() {
                self.constructs.remove(i);
                self.feat_sel = None;
                // 对话框在编辑被删元素 → 关闭;编辑其后元素 → 下标前移。
                if let Some(d) = &mut self.csys_dlg
                    && let Some(e) = d.editing
                    && e > i
                {
                    d.editing = Some(e - 1);
                }
                if self.csys_dlg.as_ref().and_then(|d| d.editing) == Some(i) {
                    self.csys_dlg = None;
                }
            }
        }
        // 被删点/轴被坐标系引用 → 去关联(数字冻结在最后一次解析值)。
        if !removed.is_empty() {
            let mut unlinked: Vec<String> = Vec::new();
            for c in &mut self.constructs {
                let hit = match c {
                    crate::construct::Construct::Csys { def: Some(d), .. } => {
                        matches!(&d.origin, crate::construct::OriginRef::ConstructPoint { name } if removed.contains(name))
                            || matches!(&d.z, crate::construct::DirRef::ConstructAxis { name } if removed.contains(name))
                    }
                    _ => false,
                };
                if hit {
                    c.drop_def();
                    if let crate::construct::Construct::Csys { name, .. } = c {
                        unlinked.push(name.clone());
                    }
                }
            }
            for n in unlinked {
                self.notify(format!("{n}:引用已删除,已去关联"));
            }
        }
        for i in sel_c {
            self.toggle_feat_sel(FeatSel::Construct(i));
        }
        if let Some(&i) = edit_c.first() {
            self.open_csys_editor(i);
        }
        if eye_p2p || del_p2p || eye_b2b || del_b2b || !eye_c.is_empty() || !del_c.is_empty() {
            self.refresh_highlight();
        }
    }

    /// 右侧属性卡片(浮动,宽 264,内容自适应高)。
    fn draw_props_card(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get();
        let screen = ui.input(|i| i.viewport_rect());
        let anchor = pos2(screen.right() - theme::SHELL_GAP - PROPS_WIDTH, Self::content_top());
        egui::Area::new(egui::Id::new("props_card"))
            .fixed_pos(anchor)
            .movable(false)
            .show(ui.ctx(), |ui| {
                crate::theme::card_frame(&t).show(ui, |ui| {
                    ui.set_min_width(PROPS_WIDTH - 2.0 * theme::SPACE_8 - 2.0);
                    // 标题行右侧加 X,关卡片(与 ribbon「视图→属性」等效)。
                    let dark = theme::is_dark();
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("属性").heading().color(t.text));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let x = ui.add(icon_image("close", 14.0, dark).tint(t.text_dim));
                            if x.clicked() {
                                self.show_props = false;
                            }
                        });
                    });
                    ui.separator();
                    let Some(doc) = &self.doc else {
                        ui.label(egui::RichText::new("(无模型)").color(t.text_dim));
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
                        ui.label(egui::RichText::new("选中").heading().color(t.text));
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
                    }
                });
            });
    }

    // ---------------- 视口叠加 ----------------

    /// 导航罗盘中心(视口右上角,照 SolidCraft)。
    fn compass_center(rect: Rect) -> Pos2 {
        pos2(rect.right() - COMPASS_SIZE / 2.0, rect.top() + COMPASS_SIZE / 2.0 - 4.0)
    }

    /// 六根带符号世界轴的屏幕投影(照 SolidCraft axis_tips),按深度由远到近排序。
    fn compass_tips(&self, c: Pos2) -> Vec<(&'static str, Vec3, Pos2, f64)> {
        let (r, u, b) = self.vp.cam.basis();
        let mut tips: Vec<_> = [("X", Vec3::X), ("-X", -Vec3::X), ("Y", Vec3::Y), ("-Y", -Vec3::Y), ("Z", Vec3::Z), ("-Z", -Vec3::Z)]
            .into_iter()
            .map(|(name, d)| {
                (
                    name,
                    d,
                    pos2(c.x + (d.dot(r) * AXIS_LEN) as f32, c.y - (d.dot(u) * AXIS_LEN) as f32),
                    d.dot(b),
                )
            })
            .collect();
        tips.sort_by(|a, b| a.3.total_cmp(&b.3));
        tips
    }

    /// 视口右上角的导航罗盘(照 SolidCraft signed_axis_view):半透明底圆 + 六向
    /// 轴尖(正轴圆盘字母,负轴空心环),点击轴尖切换标准视图。
    fn draw_compass(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let t = Tokens::get();
        let c = Self::compass_center(rect);
        let hover = ui.input(|i| i.pointer.hover_pos()).filter(|p| rect.contains(*p));
        let in_circle = hover.is_some_and(|p| p.distance(c) <= 49.5);
        let painter = ui.painter_at(rect);
        let fill = if in_circle { 1.0 } else { 0.35 };
        painter.circle(c, 49.5, t.overlay.gamma_multiply(fill), Stroke::new(1.0, t.border.gamma_multiply(0.75)));
        let tips = self.compass_tips(c);
        let hovered = hover.and_then(|p| tips.iter().enumerate().rev().find(|(_, (_, _, at, _))| at.distance(p) <= AXIS_HIT).map(|(i, _)| i));
        for (i, (name, d, at, depth)) in tips.iter().enumerate() {
            if i == 3 {
                // 原点小点画在三根后轴之上、三根前轴之下。
                painter.circle(c, 3.6, t.text.gamma_multiply(0.9), Stroke::new(1.4, t.panel));
            }
            compass_axis_tip(&painter, c, name, *d, *at, *depth, hovered == Some(i), &t);
        }
        if let Some(i) = hovered {
            let (_, d, _, _) = tips[i];
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            if ui.input(|i| i.pointer.primary_clicked()) {
                // 再点一次同一方向 = 从反方向看(照 SolidCraft)。
                let dir = if self.vp.cam.back().dot(d) > 0.9999 { -d } else { d };
                self.vp.cam = self.vp.cam.looking_from(dir);
            }
        }
    }

    /// 视口底部的 toast 卡片(照 SolidCraft:panel 底 + 描边 + 前导圆点,6 秒消失)。
    fn draw_toasts(&mut self, ui: &mut egui::Ui, rect: Rect) {
        self.toasts.retain(|t| t.alive());
        if self.toasts.is_empty() {
            return;
        }
        let t = Tokens::get();
        let painter = ui.painter_at(rect);
        let mut bottom = rect.bottom() - theme::SPACE_16;
        for toast in self.toasts.iter().rev().take(TOAST_MAX) {
            let mut text = toast.text.clone();
            if text.chars().count() > 160 {
                text = text.chars().take(160).collect::<String>() + "…";
            }
            let galley = painter.layout_no_wrap(text, fonts::medium(theme::FONT_LABEL), t.text);
            let w = galley.size().x + 8.0 + theme::SPACE_10 + theme::SPACE_10 + theme::SPACE_8;
            let h = galley.size().y.max(17.0) + theme::SPACE_8;
            let left = rect.center().x - w / 2.0;
            let r = Rect::from_min_size(pos2(left, bottom - h), vec2(w, h));
            painter.rect_filled(r, theme::RADIUS_SM, t.panel);
            painter.rect_stroke(r, theme::RADIUS_SM, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
            let dot_col = if toast.error { t.error } else { t.text };
            painter.circle_filled(pos2(r.left() + theme::SPACE_8 + 1.5, r.center().y), 3.0, dot_col);
            painter.galley(pos2(r.left() + theme::SPACE_8 + 8.0 + theme::SPACE_4, r.center().y - galley.size().y / 2.0), galley, t.text);
            bottom = r.top() - theme::SPACE_8;
        }
    }

    /// 断面分析浮窗(照 SolidCraft Section Analysis,可拖动的对话框卡片)。
    fn draw_section_dialog(&mut self, ui: &mut egui::Ui) {
        if !self.show_section_dialog {
            return;
        }
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

    /// 设置浮窗:主题 + 鼠标键位。
    fn draw_settings_window(&mut self, ui: &mut egui::Ui) {
        if !self.show_settings {
            return;
        }
        let mut open = true;
        egui::Window::new(egui::RichText::new("设置").strong())
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .show(ui.ctx(), |ui| {
                ui.label("主题");
                ui.horizontal(|ui| {
                    let dark_now = self.dark_theme.unwrap_or(theme::is_dark());
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

    /// 坐标系「定义/修改」浮窗(照 CATIA 的轴系对话框):点 chip 进视口拾取,
    /// 关联开关切换引用/数字,实时预览三轴;[确定] 落库,Esc 退拾取再关窗。
    fn draw_csys_window(&mut self, ui: &mut egui::Ui) {
        if self.csys_dlg.is_none() {
            return;
        }
        let t = Tokens::get();
        let title = if self.csys_dlg.as_ref().is_some_and(|d| d.editing.is_some()) {
            "修改坐标系"
        } else {
            "定义坐标系"
        };
        let mut open = true;
        // 闭包里只改 dlg 字段与收集动作,&mut self 的应用统一放闭包外(借用规则)。
        let mut world_z = false;
        let mut need_refresh = false;
        egui::Window::new(egui::RichText::new(title).strong())
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .show(ui.ctx(), |ui| {
                ui.set_min_width(320.0);
                let Some(dlg) = self.csys_dlg.as_mut() else { return };
                // 原点行:chip 显示引用(或未指定/拾取中),点击 chip 进/退拾取。
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("原点").color(t.text_dim));
                    let armed = dlg.armed == Some(CsysField::Origin);
                    if csys_chip(ui, &t, armed, || {
                        dlg.origin.as_ref().map(|o| o.describe()).unwrap_or_else(|| "未指定,点此拾取".into())
                    })
                    .clicked()
                    {
                        dlg.armed = if armed { None } else { Some(CsysField::Origin) };
                    }
                });
                // Z 方向行:chip + 世界 Z 快捷(照旧 Enter=世界 Z)。
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Z 方向").color(t.text_dim));
                    let armed = dlg.armed == Some(CsysField::ZDir);
                    if csys_chip(ui, &t, armed, || {
                        dlg.z.as_ref().map(|d| d.describe()).unwrap_or_else(|| "未指定,点此拾取".into())
                    })
                    .clicked()
                    {
                        dlg.armed = if armed { None } else { Some(CsysField::ZDir) };
                    }
                    if ui.button("世界Z").on_hover_text("Z 用世界 Z(相当于原来的 Enter)").clicked() {
                        world_z = true;
                    }
                });
                // 关联开关:去关联时把当前解算值落成数字,重关联时缺哪个引用拾哪个。
                let mut attached = dlg.attached;
                if ui
                    .checkbox(&mut attached, "关联几何(几何变,轴系跟着变)")
                    .on_hover_text("去关联后坐标系变成独立数字,不再随几何更新")
                    .changed()
                {
                    dlg.attached = attached;
                    need_refresh = true;
                    if attached {
                        dlg.armed = if dlg.origin.is_none() {
                            Some(CsysField::Origin)
                        } else if dlg.z.is_none() {
                            Some(CsysField::ZDir)
                        } else {
                            None
                        };
                    } else {
                        if let Some((o, _, _, z)) = csys_frame(
                            self.doc.as_ref(),
                            &self.constructs,
                            true,
                            dlg.origin.as_ref(),
                            dlg.z.as_ref(),
                            dlg.free_origin,
                            dlg.free_z,
                        ) {
                            dlg.free_origin = o;
                            dlg.free_z = z;
                        }
                        dlg.armed = None;
                    }
                }
                // 引用模式:只读结果(CATIA More 风格);独立模式:数字可编辑。
                let frame = csys_frame(
                    self.doc.as_ref(),
                    &self.constructs,
                    dlg.attached,
                    dlg.origin.as_ref(),
                    dlg.z.as_ref(),
                    dlg.free_origin,
                    dlg.free_z,
                );
                match (dlg.attached, frame) {
                    (true, Some((o, x, y, z))) => {
                        egui::Grid::new("csys_result").num_columns(2).show(ui, |ui| {
                            let v = |p: Vec3| format!("({:.3}, {:.3}, {:.3})", p.x, p.y, p.z);
                            ui.label("原点");
                            ui.label(v(o));
                            ui.end_row();
                            ui.label("X 轴");
                            ui.label(v(x));
                            ui.end_row();
                            ui.label("Y 轴");
                            ui.label(v(y));
                            ui.end_row();
                            ui.label("Z 轴");
                            ui.label(v(z));
                            ui.end_row();
                        });
                    }
                    (true, None) => {
                        ui.label(egui::RichText::new("原点或 Z 方向未定,点上面的框拾取").color(t.error));
                    }
                    (false, _) => {
                        egui::Grid::new("csys_free").num_columns(2).show(ui, |ui| {
                            ui.label("原点");
                            ui.horizontal(|ui| {
                                for (label, v) in [("X", &mut dlg.free_origin.x), ("Y", &mut dlg.free_origin.y), ("Z", &mut dlg.free_origin.z)] {
                                    ui.label(egui::RichText::new(label).color(t.text_dim).size(theme::FONT_SMALL));
                                    ui.add(egui::DragValue::new(v).speed(1.0));
                                }
                            });
                            ui.end_row();
                            ui.label("Z 方向");
                            ui.horizontal(|ui| {
                                for (label, v) in [("X", &mut dlg.free_z.x), ("Y", &mut dlg.free_z.y), ("Z", &mut dlg.free_z.z)] {
                                    ui.label(egui::RichText::new(label).color(t.text_dim).size(theme::FONT_SMALL));
                                    ui.add(egui::DragValue::new(v).speed(0.05));
                                }
                            });
                            ui.end_row();
                        });
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("确定").clicked() {
                        self.commit_csys();
                    }
                    if ui.button("取消").clicked() {
                        self.close_csys_dialog();
                    }
                });
            });
        if world_z
            && let Some(d) = &mut self.csys_dlg
        {
            d.z = Some(crate::construct::DirRef::Fixed(Vec3::Z));
            d.armed = None;
            need_refresh = true;
        }
        if need_refresh {
            self.refresh_highlight();
        }
        if !open {
            self.close_csys_dialog();
        }
    }
}

/// 坐标系对话框的引用 chip:点击进视口拾取,拾取中高亮。
fn csys_chip(ui: &mut egui::Ui, t: &Tokens, armed: bool, text: impl FnOnce() -> String) -> egui::Response {
    let text = if armed { "拾取中…(点视口)".to_string() } else { text() };
    let btn = egui::Button::new(egui::RichText::new(text).size(theme::FONT_BODY))
        .fill(if armed { t.accent_soft } else { t.field })
        .stroke(if armed {
            egui::Stroke::new(1.0, t.accent)
        } else {
            egui::Stroke::new(1.0, t.border)
        })
        .corner_radius(theme::RADIUS_SM)
        .min_size(vec2(200.0, 22.0));
    ui.add(btn)
}

/// 文件菜单的一行:占满菜单宽,悬停/按下高亮,文字左对齐(照应用栏按钮的手绘方式;
/// egui 0.36 的 Area 无界 Ui 里 justify 会撑爆,不能用 selectable_label + justify)。
fn menu_item(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let t = Tokens::get();
    let w = ui.available_width();
    let h = 26.0;
    let resp = ui.allocate_response(vec2(w, h), egui::Sense::click());
    if resp.hovered() || resp.is_pointer_button_down_on() {
        ui.painter().rect_filled(resp.rect, theme::RADIUS_SM, t.hover);
    }
    ui.painter().text(
        pos2(resp.rect.left() + 8.0, resp.rect.center().y),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(theme::FONT_BODY),
        t.text,
    );
    resp
}

/// 导航罗盘的一根轴尖(照 SolidCraft navigation::axis_tip):正轴 = 轴杆 + 实心圆盘
/// + 字母,负轴 = 空心环;背离视线减淡,悬停变白。
fn compass_axis_tip(painter: &egui::Painter, c: Pos2, name: &str, d: Vec3, at: Pos2, depth: f64, hovered: bool, t: &Tokens) {    let col = if d.x != 0.0 {
        t.triad[0]
    } else if d.y != 0.0 {
        t.triad[1]
    } else {
        t.triad[2]
    };
    let positive = !name.starts_with('-');
    let light = t.panel.r() > 128;
    // 正轴用全色,负轴和背离视线的轴减淡。
    let ink = if !positive || depth < -0.01 { col.gamma_multiply(0.55) } else { col };
    let radius = if positive { 11.0 } else { 4.8 };
    let direction = (at - c).normalized();
    let width = 3.6;
    // 负轴杆止于环外缘;正轴杆伸到盘下。
    let end = if positive { at - direction * radius } else { at - direction * (radius + 1.2) };
    painter.line_segment([c, end], Stroke::new(width, ink));
    painter.circle_filled(c, width / 2.0, ink);
    if positive {
        painter.circle_filled(at, radius, col);
    }
    if hovered {
        painter.circle_filled(at, if positive { radius } else { radius + 1.2 }, t.on_accent);
        // 亮主题下白盘会没进罗盘底,加一圈轴色描边。
        if light && positive {
            painter.circle_stroke(at, radius - 1.0, Stroke::new(2.0, col));
        }
    }
    if positive {
        // 悬停(白盘)时字母用轴色,平时用 accent 上的白。
        let letter = if hovered { col } else { t.on_accent };
        let galley = painter.layout_no_wrap(name.to_owned(), fonts::bold(theme::FONT_COMPASS), letter);
        let origin = at - galley.size() / 2.0;
        painter.galley(origin, galley, letter);
    }
    if !positive {
        painter.circle_stroke(
            at,
            radius,
            Stroke::new(
                2.4,
                if !hovered {
                    ink
                } else if light {
                    col
                } else {
                    t.on_accent
                },
            ),
        );
    }
}

/// 坐标系对话框状态的解算结果(预览/确定/切换关联共用):关联走引用现算,独立用数字。
#[allow(clippy::too_many_arguments)]
fn csys_frame(
    doc: Option<&crate::scene::Doc>,
    constructs: &[crate::construct::Construct],
    attached: bool,
    origin: Option<&crate::construct::OriginRef>,
    z: Option<&crate::construct::DirRef>,
    free_origin: Vec3,
    free_z: Vec3,
) -> Option<(Vec3, Vec3, Vec3, Vec3)> {
    if attached {
        crate::construct::resolve_csys(
            doc?,
            constructs,
            &crate::construct::CsysDef { origin: origin?.clone(), z: z?.clone() },
        )
    } else {
        let (x, y) = crate::construct::csys_axes(free_z)?;
        Some((free_origin, x, y, free_z))
    }
}

/// 树节点行:checkbox 显隐、名字可点选整个体;收集切换/选中的路径。
/// 特征列表一行:眼睛 + 类型图标 + 名字 + 弱色 ×;返回 (眼睛, 删除, 名字单击, 名字双击) 四次交互。
fn feature_row_ui(
    ui: &mut egui::Ui,
    t: &Tokens,
    dark: bool,
    icon: &str,
    label: &str,
    visible: bool,
    selected: bool,
) -> (bool, bool, bool, bool) {
    let mut out = (false, false, false, false);
    ui.horizontal(|ui| {
        ui.set_min_height(TREE_ROW_HEIGHT);
        // 眼睛按钮(可见=亮,隐藏=暗),照 tree_node_ui。
        let eye = ui.add(
            icon_image("eye", 15.0, dark).tint(if visible {
                t.text.gamma_multiply(0.85)
            } else {
                t.text_dim.gamma_multiply(0.5)
            }),
        );
        if eye.clicked() {
            out.0 = true;
        }
        ui.add(icon_image(icon, 16.0, dark));
        let mut name = egui::RichText::new(label).size(theme::FONT_BODY);
        if selected {
            name = fonts::semibold_text(theme::FONT_BODY, label).color(t.accent_ink);
        }
        let resp = ui.selectable_label(selected, name);
        if resp.clicked() {
            out.2 = true;
        }
        if resp.double_clicked() {
            out.3 = true;
        }
        // × 删除(常驻弱色,右对齐)。
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let del = ui.add(icon_image("close", 12.0, dark).tint(t.text_dim.gamma_multiply(0.7)));
            if del.clicked() {
                out.1 = true;
            }
        });
    });
    out
}

fn tree_node_ui(
    ui: &mut egui::Ui,
    node: &Node,
    path: &[usize],
    toggles: &mut Vec<Vec<usize>>,
    selects: &mut Vec<Vec<usize>>,
    sel: Option<&Sel>,
    dark: bool,
    t: &Tokens,
) {
    let id = format!("node{}", path.iter().fold(0usize, |a, b| a * 1000 + b));
    ui.push_id(id, |ui| {
        ui.horizontal(|ui| {
            ui.set_min_height(TREE_ROW_HEIGHT);
            // 眼睛按钮(可见=亮,隐藏=暗):点击切换显隐。
            let eye = ui.add(
                icon_image("eye", 15.0, dark).tint(if node.visible {
                    t.text.gamma_multiply(0.85)
                } else {
                    t.text_dim.gamma_multiply(0.5)
                }),
            );
            if eye.clicked() {
                toggles.push(path.to_vec());
            }
            // 体/装配图标。
            let kind = if node.mesh.is_some() { "body" } else { "component" };
            ui.add(icon_image(kind, 16.0, dark));
            let selected = sel.is_some_and(|s| s.path == path);
            let mut name = egui::RichText::new(&node.name).size(theme::FONT_BODY);
            if node.mesh.is_none() {
                name = name.weak();
            }
            if selected {
                name = fonts::semibold_text(theme::FONT_BODY, &node.name).color(t.accent_ink);
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
                        tree_node_ui(ui, child, &p, toggles, selects, sel, dark, t);
                    }
                });
        }
    });
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
        // 主题:暗/亮/跟随系统;主题变了重建场景(视口背景/网格换色)。
        let dark = match self.dark_theme {
            Some(true) => {
                ui.ctx().set_theme(egui::ThemePreference::Dark);
                true
            }
            Some(false) => {
                ui.ctx().set_theme(egui::ThemePreference::Light);
                false
            }
            None => {
                ui.ctx().set_theme(egui::ThemePreference::System);
                ui.ctx().system_theme().is_none_or(|t| t == egui::Theme::Dark)
            }
        };
        theme::set_dark(dark);
        theme::apply(ui.ctx());
        let dark_now = theme::is_dark();
        if self.prev_dark != Some(dark_now) {
            self.prev_dark = Some(dark_now);
            if self.doc.is_some() {
                self.scene_dirty = true;
            }
        }
        self.poll_load();
        self.poll_measure();
        self.handle_drop(ui);
        // Esc:坐标系对话框打开时先退拾取、再关窗;否则取消激活的工具。
        // Enter = 对话框确定。数字框编辑中(focus 在输入框)时按键不抢。
        if self.csys_dlg.is_some() && ui.memory(|m| m.focused().is_none()) {
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                if self.csys_dlg.as_ref().and_then(|d| d.armed).is_some() {
                    if let Some(d) = &mut self.csys_dlg {
                        d.armed = None;
                    }
                } else {
                    self.close_csys_dialog();
                    self.notify("已取消坐标系");
                }
            }
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.commit_csys();
            }
        } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) && self.tool.take().is_some() {
            self.tool_prompt.clear();
            self.notify("工具已取消");
            self.refresh_highlight();
        }
        // 选中对象的质量属性/面信息懒加载(后台)。
        let lazy = self.sel.as_ref().map(|s| (s.path.clone(), s.face));
        if let Some((path, face)) = lazy {
            match face {
                Some(f) => self.request_face(path, f),
                None => self.request_mass(path),
            }
        }
        // 布局:应用栏 → ribbon → 中央视口(全出血)→ 浮动卡片 → 浮窗 → 缩放区。
        if self.scene_dirty {
            self.refresh_scene();
        }
        self.draw_app_bar(ui);
        self.draw_ribbon(ui);
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let (rect, response) =
                    ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
                self.handle_input(&response);
                // 指针悬在罗盘上时点击归罗盘,不拾取。
                let hover = ui.input(|i| i.pointer.hover_pos());
                let over_compass = hover
                    .filter(|p| rect.contains(*p))
                    .is_some_and(|p| {
                        let c = Self::compass_center(rect);
                        p.distance(c) <= 49.5
                            || self
                                .compass_tips(c)
                                .iter()
                                .any(|(_, _, at, _)| at.distance(p) <= AXIS_HIT)
                    });
                if response.clicked() && !over_compass {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let (x, y) = (pos.x - rect.left(), pos.y - rect.top());
                        let hit = self.pick_hit(x, y);
                        // 坐标系对话框拾取中:点击填入对话框,不选对象。
                        if !self.csys_pick(hit.as_ref(), x, y) && !self.tool_click(hit.as_ref(), x, y) {
                            self.set_sel(hit.map(|h| Sel { path: h.path, face: Some(h.face) }));
                        }
                    }
                } else if response.double_clicked() && self.tool.is_none() && self.csys_dlg.is_none() && !over_compass {
                    // 双击:从面升级为整体选中。
                    if let Some(pos) = response.interact_pointer_pos()
                        && let Some(hit) =
                            self.pick_hit(pos.x - rect.left(), pos.y - rect.top())
                    {
                        self.set_sel(Some(Sel { path: hit.path, face: None }));
                    }
                }
                self.vp.show(ui, rect);
                self.draw_compass(ui, rect);
                // 叠加标签(测量结果/质心/构造)投影到屏幕。to_screen 给的是视口内坐标,
                // 绘制要加视口左上角。
                for (mid, text, color, offset) in self.overlay_labels() {
                    if let Some((sx, sy, _)) = self.vp.cam.to_screen(
                        mid,
                        f64::from(self.vp.size[0]),
                        f64::from(self.vp.size[1]),
                        self.vp.radius,
                    ) {
                        ui.painter().text(
                            egui::pos2(rect.left() + sx as f32 + offset[0], rect.top() + sy as f32 + offset[1]),
                            Align2::CENTER_CENTER,
                            text,
                            fonts::medium(theme::FONT_LABEL),
                            color,
                        );
                    }
                }
                // 工具进行中/坐标系拾取中的常驻提示:视口左上角(照 SolidCraft 的状态文字)。
                let csys_hint = self.csys_dlg.as_ref().and_then(|d| d.armed).map(|f| match f {
                    CsysField::Origin => "坐标系:点原点(顶点/圆弧圆心/圆类面中心/面上;Esc 退拾取)",
                    CsysField::ZDir => "坐标系:点 Z 方向(直边/圆弧边/圆柱类面)",
                });
                if let Some(hint) = csys_hint {
                    ui.painter().text(
                        pos2(rect.left() + theme::SPACE_12, rect.top() + theme::SPACE_12),
                        Align2::LEFT_TOP,
                        hint,
                        fonts::medium(theme::FONT_LABEL),
                        Tokens::get().accent,
                    );
                } else if self.tool.is_some() && !self.tool_prompt.is_empty() {
                    ui.painter().text(
                        pos2(rect.left() + theme::SPACE_12, rect.top() + theme::SPACE_12),
                        Align2::LEFT_TOP,
                        &self.tool_prompt,
                        fonts::medium(theme::FONT_LABEL),
                        Tokens::get().accent,
                    );
                }
                self.draw_toasts(ui, rect);
                if let Some(name) = self.loading.as_ref() {
                    ui.painter().text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        format!("正在导入 {name} …"),
                        fonts::medium(16.0),
                        Color32::from_gray(180),
                    );
                } else if self.doc.is_none() {
                    ui.painter().text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        "把 STEP / IGES / STL / 3MF / OBJ / glTF 文件拖进来",
                        fonts::medium(18.0),
                        Color32::from_gray(120),
                    );
                }
            });
        self.draw_browser_card(ui);
        if self.show_props {
            self.draw_props_card(ui);
        }
        self.draw_section_dialog(ui);
        self.draw_settings_window(ui);
        self.draw_csys_window(ui);
        if self.file_menu_open {
            self.draw_file_menu(ui);
        }
        titlebar::resize_zones(ui);
    }
}

impl Default for CadViewApp {
    fn default() -> Self {
        Self::with_viewport(Viewport::new(), Vec::new(), Some(true), 0, 2)
    }
}
