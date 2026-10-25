# CADViewCraft 可行性评估(v2:复用 SolidCraft crate)

> 目标:用 Rust + egui 做 Mayo 风格 CAD 查看器。**功能裁剪为:打开 STEP(含装配树)、模型树显隐、断面(剖切)、测量。爆炸视图不做。**
> 实现方式(v2,2026-10-24 与用户确认):不再走 CLI 外壳,直接把 [SolidCraft](https://github.com/tangency-labs/solidcraft)(已 clone 到 `D:\study.git\solidcraft`)的底层 crate 作为库依赖;UI 参考 Mayo。
> v1(CLI 桥接方案)见本文末尾附注。

## 1. 结论

**可行,工作量约 4~5 人周(MVP),比 v1 方案的 7~10 人周明显缩短,能力上限更高。**

调研发现(逐 Cargo.toml 与源码核实):

- SolidCraft 分层干净:`geom / kernel / render / gpu` 四个 crate **完全不依赖** doc/engine/sketch,可当纯几何与渲染库直接用;`kernel::step_import_shared()` 直接产出装配树(层级/名称/颜色/单位→mm),**不需要 io 层**(io 会把 doc→sketch 整条参数化文档链拖进来)。
- 需要的四个功能在 SolidCraft 里**全部现成**:
  - **STP 导入**:`kernel::step_import_shared(text) -> StepImport { bodies, tree, warnings, schema, system }`,其中 `StepNode { name, transform(4×4, mm), bodies, children }` 就是装配树;`ImportedBody { name, body, color, ... }`。
  - **显示网格**:`Body::tessellate(弦差mm) -> geom::Mesh`,带 `tri_face`(三角形→B-rep 面号)和 `edges`(边折线)——**能做面级拾取与高亮**,这是 v1 方案(纯网格)做不到的。
  - **测量**:`kernel::measure`(体积/表面积/质心/包围盒/拓扑统计)、`body_distance`(体-体精确距离,在 B-rep 上投影)、`Body::faces()/edges()`(逐面面积/法向、逐边长度)、`face_surface`(解析面类型:平面/圆柱/圆锥/球/环)。
  - **断面**:`render::Scene` 有 `Section { normal, d }`(GPU 裁剪),gpu crate 有截面 cap + hatch(剖面填充线)渲染管线——即渲染级剖切现成;几何级截面(提取截面线)kernel 无现成 API,可后续用 `split_by_plane` 补。
- **不能在 ui-egui 上动手术**:ui-egui 292 个文件中 198 个 import engine(参数化引擎),删 engine = 重写整个 UI。所以"删减"的正确姿势是:CADViewCraft 作为薄壳,**只拉 `geom + kernel + render + gpu` 进依赖树**,engine/sketch/doc/io/ui-egui/mcp/space-mouse/web 全不进。

## 2. 依赖方案

CADViewCraft 根 `Cargo.toml`:

```toml
[dependencies]
solidcraft-geom   = { path = "../solidcraft/crates/geom" }
solidcraft-kernel = { path = "../solidcraft/crates/kernel" }
solidcraft-render = { path = "../solidcraft/crates/render" }
solidcraft-gpu    = { path = "../solidcraft/crates/gpu" }

[patch.crates-io]   # 必须复制,solidcraft 仓库位置变了要跟着改
truck-geometry = { path = "../solidcraft/vendor/truck-geometry" }
truck-shapeops = { path = "../solidcraft/vendor/truck-shapeops" }
truck-stepio   = { path = "../solidcraft/vendor/truck-stepio" }
truck-meshalgo = { path = "../solidcraft/vendor/truck-meshalgo" }
matext4cgmath  = { path = "../solidcraft/vendor/matext4cgmath" }
egui-wgpu      = { path = "../solidcraft/vendor/egui-wgpu" }
```

**版本对齐**:SolidCraft 生态绑 egui/egui-wgpu **0.36**(vendor 的 egui-wgpu 补丁是 0.36 的,处理 MSAA 采样数),CADViewCraft 从 0.37 降到 **0.36 + eframe 0.36**,gpu 集成胶水可直接参考 `apps/solidcraft/src/main.rs`(初始化顺序)与 `ui-egui/src/gpu.rs`(paint callback 安装)。

## 3. 架构

```
CADViewCraft (eframe 0.36, Mayo 布局: 菜单栏/左模型树/中视口/右属性/底状态栏)
├── importer/  读文件→编码探测→kernel::step_import_shared
│              (编码探测可抄 io 层的 chardetng/GBK/Shift_JIS 处理)
├── scene/     StepImport.tree → 场景节点树(名字/变换/可见性开关)
│              每体 body.tessellate(弦差) → Mesh;节点显隐只改标志位
├── viewport/  gpu::view::Renderer 以 egui_wgpu paint callback 画 GpuScene;
│              相机/适配/标准视角;拾取:射线→三角形→tri_face→面号
├── measure/   属性面板:kernel::measure;
│              交互:点距(视口内)、体-体距离 body_distance、
│              点-面距离(用 FaceSurface::nearest 自建)、面夹角(平面)
└── section/   剖切平面 UI(法向/偏移,或过三点)→ Scene.section
```

## 4. 工作量(1 名熟悉 Rust 的开发,全职)

| 里程碑 | 内容 | 估算 |
|---|---|---|
| M0 对齐 | path 依赖 + vendor 补丁 + egui 降 0.36,骨架适配 | 2~3 天(骨架已建) |
| M1 打开+显示 | STEP→tessellate→GpuScene→渲染;Mayo 式布局 | 1~1.5 周 |
| M2 树与显隐 | 装配树 UI、逐节点显隐、选择高亮(体/面级) | 3~5 天 |
| M3 断面 | 剖切平面交互,cap+hatch(gpu 现成) | 2~4 天 |
| M4 测量 | 属性面板 + 交互测量(点距/体距/点面距/面角) | 1~1.5 周 |
| M5 打磨 | 视图工具、IGES(kernel::iges_to_step)、最近文件、错误处理 | 3~5 天 |
| **MVP 合计** | | **约 4~5 人周** |

## 5. 风险

1. **gpu crate 无文档**,`view.rs` 约 3300 行,胶水要读代码;缓解:apps/solidcraft 与 ui-egui 有完整可抄的用例。
2. **vendor 补丁是全局的**:必须在 CADViewCraft workspace 根复制 `[patch.crates-io]`;solidcraft 目录移动会断(用相对路径,文档里写明)。
3. **egui 0.36↔0.37 API 差异**:骨架需小改(0.36 已有 App::ui/logic 拆分,Panel/Menu API 需对一遍)。
4. **编码问题**:kernel 层收 STEP 文本,GBK/Shift_JIS 编码的文件要自己做探测(io 层有现成逻辑可抄)。
5. **上游同步**:我们不改 solidcraft 源码(零分叉),上游更新只需 `cargo update` + 回归;若上游改 GPU 接口,胶水层要跟。
6. 几何级截面(提取截面线/面)无现成 API,MVP 用渲染级剖切顶上;真需要时用 `split_by_plane` + 切面轮廓补,另加 3~5 天。

## 6. 验证结果(2026-10-24,M0.5 已完成 ✅)

1. ✅ **path 依赖 + vendor 补丁接线成功**:CADViewCraft 根 Cargo.toml 挂 `../solidcraft/crates/{geom,kernel,render,gpu}` + `[patch.crates-io]` 指向 `../solidcraft/vendor/*`,egui/eframe 降至 0.36,`cargo build --offline` 通过。骨架代码(App::ui、egui::Panel、MenuBar/MenuButton)在 0.36 下**零修改编译通过**。
2. ✅ **STEP 导入实测**(`--selftest`,debug 构建,样例 `solidcraft/tests/step-conformance/grooved_shaft.step`,AP214,800KB):
   - `step_import_shared`:1 body,名称/路径正确,装配树 1 根,0 警告
   - `tessellate(0.1mm)`:144 372 顶点 / 48 124 三角形 / **753 个 B-rep 面号** / 2 255 条边折线,2.5s —— tri_face/edges 齐全,面级拾取的基础成立
   - `measure`:体积 153 518 mm³、表面积 38 059 mm²、质心落在轴线上,数值合理
3. ✅ **release 性能实测**(同一样例):导入 **1.55s**、细分 0.1mm **157ms**、精细测量 **4.8s**(debug 为 39s/2.5s/88s)。打开文件的耗时完全可接受;`measure` 后台线程算即可,无需另寻方案。
4. ⏳ 待做:gpu crate 离屏渲染验证(抄 cli snapshot 链路,归入 M1);多体装配 STEP 保真度验证(仓库样例都是单件,需真实工业装配文件)。

## 7. M1 完成记录(2026-10-24 ✅)

视口渲染管线已接通,结构:

- `src/scene.rs`:STEP → 节点树(实例变换烘焙进顶点,颜色取文件外观/默认钢蓝灰)→ `render::Scene`(着色体 + B-rep 边线 + 地面网格 + 背景)
- `src/viewport.rs`:`egui_wgpu::CallbackTrait` 直通 `gpu::view::Renderer`(照抄 ui-egui/src/gpu.rs 的 split-target 模式:场景目标带 depth24-stencil8 + MSAA 4x,窗口目标无深度);相机用 `solidcraft_render::Camera`(Z-up,正交默认)
- `src/app.rs`:Mayo 布局(菜单栏/左模型树/中视口/右属性/底部状态栏),后台线程加载,拖放 + 命令行参数 + rfd 文件对话框打开,树节点 checkbox 显隐
- 验证:`--selftest --png` 离屏渲染(与视口同一 Scene→GPU 链路),100 孔板渲染质量达标(孔内壁明暗/圆角/边线正确),样例图见 `docs/img/`;GUI 冒烟测试进程稳定,clippy 零告警

踩过的 API 坑(egui 0.36):`DroppedFile` 是 trait(`Arc<dyn DroppedFile>`,取 `f.path() -> &Path`);`Panel::default_size(f32)`;`Response::ctx` 是字段;`eframe::egui_wgpu` 才是(打了补丁的)egui_wgpu;`Vec3` 在 `solidcraft_geom` 根。

## 8. M2 完成记录(2026-10-24 ✅)

选择与高亮:

- `src/pick.rs`:Möller–Trumbore 双面射线求交,遍历烘焙网格(tri_face 面号),返回 {节点路径, 面号, 命中点, 距离}。13k 三角形实测 1.05ms——交互级。
- `viewport.rs`:`hl_slot`/`hl_key` 高亮通道(Frame 的 hl 字段,与 solidcraft 相同;渲染器自带深度偏移)。
- `app.rs`:单击选面(平涂亮橙 `SEL_FACE`)、双击升级选体(带光照的蓝 `SEL_BODY`);模型树 selectable_label 同步选中(点击树行选体);属性面板显示选中对象/类型/面号;选中节点被隐藏时自动取消选中。
- 验证:selftest 图心拾取命中 face 62(孔内壁)✓;clippy 零告警;GUI 冒烟存活。高亮的窗口内视觉效果待人工确认(离屏 Capturer 无 highlight 通道)。

## 9. M3 完成记录(2026-10-25 ✅)

断面:

- 视口路径走 `Frame.clip` uniform `[n.x, n.y, n.z, n·o]`(`[0;4]`=关),shader 约定 **discard where n·p > d**;cap 颜色/剖面线开关走 `Frame.cap` 的 alpha 位。改动是 uniform 级,**不触发场景重建**。
- `app.rs`:右面板"断面"区(启用/方向 ±X±Y±Z 预设/剖切深度滑条/剖面填充线开关);`section_clip()` 由包围盒 8 角投影算 d(cut=0 不剖,1 全剖)。
- `pick.rs`:三点全在剖掉侧(n·p > d)的三角形不参与命中。
- 验证:全剖实验(模型消失、pick=None)确认 uniform 通路;轴件纵剖(法向 -Y)PNG 显示橙色斜线剖面填充贯穿全长(docs/img/selftest_section.png);pick 命中保留侧 ✓;clippy 零告警;GUI 冒烟存活。
- 备注:捕获路径(Scene.clip)与视口路径(Frame.clip)是两条独立通路,均验证。

## 10. M4 完成记录(2026-10-25 ✅)

测量(Mayo 四大核心功能收官):

- **点-点距离**:菜单"测量 → 点-点距离"激活工具,视口点两个点 → 绿色连线 + 端点十字(高亮场景 overlay)+ 屏幕标签,Esc 取消。
- **体-体距离**:同上点两个体 → 后台 `kernel::body_distance`(DistanceShape 精确 B-rep 投影,limit 用两体尺寸和)→ 连线 + 标签(非精确结果带 ≈ 号)。
- **质量属性**:双击选体 → 后台 `kernel::measure` → 属性面板显示体积/表面积/包围盒/质心(懒加载,单 worker 队列)。
- **面信息**:单击选面 → 后台 `Body::faces(chord)` → 面积/法向/质心。
- 实现:scene::Node 保留 kernel `Body`(cheap clone,Send+Sync);app 单 worker + mpsc;测量图形并入 hl_slot 高亮场景;标签用 `Camera::to_screen` 投影。
- 验证:self-distance 自检 `Some(0.0)`(body_distance 调用方式正确);measure/faces 与 CLI selftest 数据一致;clippy 零告警;GUI 冒烟存活。
- kernel API 坑:`faces()` 返回 `Result<Vec<FaceInfo>>`;DistanceShape/body_distance 从 kernel 根导出(模块私有)。

## 11. M5 完成记录(2026-10-25 ✅)——全部里程碑收官

- **中文乱码修复**:egui 默认字体无 CJK 字形。`install_cjk_font()` 从系统加载微软雅黑 `msyh.ttc`(FontData.index 取第 0 face,退路 simhei/Deng/simsun),插入 Proportional+Monospace 两族首位。日志确认加载成功。
- **IGES**:扩展名路由,.igs/.iges 先 `kernel::iges_to_step`(带警告回传)再走 STEP 导入。注:仓库无 IGES 样例,转换未实测,首次用真实文件时验证。
- **视图菜单**:适配视图 + 前/后/左/右/俯/仰/等轴测(`solidcraft_render::StandardView.angles()`)。
- **坐标轴 gizmo**:视口左下角,世界 XYZ 按 `cam.basis()` 投影,红绿蓝 + 字母。
- **最近文件**:File 菜单"最近文件"子菜单(去重置顶,存 10 条),eframe persistence(ron)跨会话持久化。
- 依赖注意:persistence 需要 ron;本机网络下 ron 的 **sparse index 缓存**要手动搬运(规则:≤3 字符名 `{len}/{首字符}/{名}`,≥4 字符 `{前2}/{接2}/{名}`,源是镜像内容等价可直接拷)。

## 12. 首轮验收反馈修复(2026-10-25)

用户试用 Part1.stp 后的三项反馈:

- **工具栏**(照 SolidCraft/Mayo):菜单栏下方一行——打开 | 适配/前视/等轴测 | 点距/体距(激活态高亮)/清除测量 | 断面(开关态高亮)。动作与菜单共用 helper(open_dialog/start_tool_*/toggle_section)。
- **断面按钮**:工具栏可开关(默认自顶向下剖一半),细调仍在右面板。
- **拾取点即时高亮**:① 工具收集中已点进度即画(点=十字标记;体=浅蓝平涂 TOOL_PENDING);② 修了端点十字是世界尺寸导致缩小时不可见的问题——改为按视口高度换算、恒定约 9px;③ 顺带修复显隐切换会重置相机的问题(refresh_scene 只在 fit_pending 时适配;适配视图独立为 fit_view)。

## 13. 界面改版(2026-10-25,照 SolidCraft 风格)

- **移除传统菜单栏**(文件/视图/测量/帮助),改为 **ribbon 分组工具栏**:`文件(打开/最近/退出) | 视图(适配/前视/等轴测) | 检查(点距/体距/清除/质心/属性/断面)`,组名在按钮下方(照 Inspect 分组样式)。
- **断面参数行**:断面激活时 ribbon 下出现第二行(方向 ±Z±X±Y/深度滑条/剖面填充线),右面板不再有断面组。
- **质心按钮**:开启后选中体显示质心十字标记(紫色)+ 屏幕标签"质心 (x, y, z)"(质量属性懒加载)。
- **属性按钮**:开关右侧属性面板。
- **坐标轴 gizmo 移到视口右上角**(照 SolidCraft)。
- 叠加标签泛化为 `overlay_labels()`(点距/体距/质心)。
- 环境事故:D 盘满(100%),清理 `target/*/incremental` 后恢复;`cadviewcraft.exe` 被运行中的实例占用导致链接失败,先 taskkill 再编。

## 14. ribbon 布局两连修(2026-10-25)

- **按钮竖排成列**:`ribbon_group` 少了 horizontal 包裹(vertical 布局里按钮逐行换行),补上。
- **`SelectableLabel` 在 egui 0.36 已移除**(lib.rs 里的同名项是 `UiKind` 枚举成员):改用 `egui::Button::selectable(selected, text)` + `ui.add_sized([56,34], ...)`,激活态/悬停提示齐全,按钮尺寸与 ribbon_button 统一。
- 工具按钮改为再次点击可取消(点距/体距激活时再点一次关闭)。
- 教训升级:**凡构建前 exe 可能被占用,先 taskkill;构建必须看到 Finished 才算数**——之前 release 构建失败被忽略,导致旧 exe 被反复启动,浪费一轮排查。

## 15. 图标加载修复(2026-10-25)

图标全部显示红色警告三角(egui 的加载失败占位)。根因:`egui_extras::install_image_loaders` 的每个 loader 都有独立 feature——**`file://` loader 需要 `file` feature**,只开 `["svg","image"]` 时 URI 无人处理。补齐为 `features = ["svg", "image", "file"]`。

最终方案:SolidCraft 的 SVG 母版复制到 `assets/icons/`(open/fit/lookat/iso_analysis/measure/inspect/close/center_of_mass/section/settings 十枚),`egui_extras::install_image_loaders` + `Image::new(file:///CARGO_MANIFEST_DIR/assets/icons/*.svg)` 加载;按钮为图标上/文字下的自绘 `ribbon_icon_button`(选中底色+描边、悬停底色)。属性面板默认收起,ribbon"属性"按钮开关。

## 16. 图标着色 + 树图标化(2026-10-25)

- **警告三角根因之二**:flat 目录图标(fit/lookat/eye 等)用 CSS `currentColor`,resvg 无上下文时渲成黑色(暗背景不可见)。SolidCraft 靠自家调色板注入;我们改为加载时把 `currentColor` 替换为暗主题 ink 色 `#c9d4e0`,经 `Image::from_bytes` 字节直供(绕开 file loader,`cache://icons/名.svg` 虚拟 URI),OnceLock 全量缓存。
- **模型树照 SolidCraft Browser 改造**:checkbox → 眼睛图标(可见亮/隐藏暗灰,点击切换),节点加体(body)/装配(component)图标。
- 过程事故:python 用 `find("}")` 删函数撞上 format! 字符串里的 `{}`,把文件尾部切坏——逐层修复后恢复。教训:**批量文本手术禁止用裸 `find("}")` 定位函数边界;改函数用整块标记切片 + 操作后立即 cargo build 验证**。

## 17. M6:设置/主题/鼠标配置 + 断面浮窗(2026-10-25)

- **断面分析浮窗**(照 SolidCraft Section Analysis):点 ribbon"断面"弹出 `egui::Window`——方向预设 + 反转按钮、深度滑条(百分比)、剖面填充线开关、关闭断面/取消按钮;浮窗 X 也关闭。**ribbon 第二行参数行删除**,不再引起布局跳动;关浮窗 = 关断面。
- **设置浮窗**(ribbon 检查组新增"设置"图标按钮):主题三选(跟随系统/暗色/亮色,即时生效)、鼠标键位配置(旋转/平移各一个下拉:左/中/右键,缩放固定滚轮)。
- **持久化**:主题与鼠标键位存 eframe storage,与最近文件一致。
- **图标双主题**:SVG 注入色按主题两份(暗=#c9d4e0 亮墨 / 亮=#3b4351 深墨),`icon_image(name, size, dark)` 按 `ctx.theme()` 取用;"属性"按钮图标换 appearance.svg。
- egui 0.36 API:`ThemePreference::{Dark,Light,System}`(无 FollowSystem);`Theme` 判暗用 `matches!(ctx.theme(), Theme::Dark)`。

## 18. 主线切换(2026-10-25,最终状态)

**自研版(CADViewCraft,M0–M5 + 各项反馈修复)就此封存,不再维护。** 主线改为精简版 SolidCraft:`D:\study.git\solidcraft` 的 **`viewer` 分支**(git 可回退,提交记录:页签裁剪、版本 0.0.1)。

- 裁剪:ribbon 页签 7→2(INSPECT/VIEW),删上下文页签,默认 INSPECT;MEASURE 组(Masure/Mass Center/Properties)+ SECTION 组即全部检查功能
- 版本约定:**viewer 分支 workspace version 从 0.0.1 起,每次发布构建手动 +0.0.1**(Cargo.toml 一处,关于对话框/标题同步显示);窗口标题 "CADViewCraft 0.0.1"
- 启动:`D:\study.git\solidcraft	argetelease\solidcraft.exe [模型.stp]`
- 后续精简原则(用户定):SolidCraft 有的直接全盘保留,**不需要的功能先不删、后期再改**

## 附:本机网络备注

crates.io 的 index/static 间歇被墙,rsproxy 可用但偶发 504/慢;且 rsproxy CDN 关连接时 Windows schannel 会报"failed to decrypt data"使 curl 返回非零——**用 `gzip -t` 校验文件完整性而不是看 curl 退出码**。缺包时手动从 `https://rsproxy.cn/api/v1/crates/<name>/<ver>/download` 拉进 `~/.cargo/registry/cache/index.crates.io-*/` 后 `cargo build --offline` 即可。

---

## 附:v1 方案(CLI 桥接,已放弃)

原计划 CADViewCraft 壳 + spawn `solidcraft-cli`(import→glTF 导出、eval 测量)。放弃原因:每次交互过进程墙、glTF 导出丢失 B-rep 信息(做不了面级拾取)、MVP 估 7~10 人周反而更长。v1 全文见 git 历史。
