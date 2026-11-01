# CADViewCraft

> **⚠ 主线已切换(2026-10-25):本项目(自研版)封存,当前主线为精简版 SolidCraft —— `D:\study.git\solidcraft` 仓库的 `viewer` 分支,版本 0.0.10 起。**
> 启动:`D:\study.git\solidcraft\target\release\solidcraft.exe [模型.stp]`,版本号每次发布在 `solidcraft/Cargo.toml` 手动 +0.0.1。
> 以下为自研版的历史文档,仅作备选。


Rust + egui 编写的轻量 3D CAD 查看器。打开 STEP/IGES/网格模型,查看装配结构、做断面剖切、进行测量。

**完整功能与使用方法见 [docs/使用说明.md](docs/使用说明.md)。**

## 启动

前置条件:本项目依赖同级的 `../solidcraft` 仓库(path 依赖 + vendor 补丁,详见 `docs/feasibility.md` 第 2 节)。

**首次使用,先构建一次:**

```bash
cd D:\study.git\CADViewCraft
cargo build --release
```

**日常启动(任选其一):**

```bash
# 1. 命令行直接打开模型(推荐)
target\release\cadviewcraft.exe D:\study.git\testdata\Part1.stp

# 2. 先启动空窗口,再把文件拖进窗口
target\release\cadviewcraft.exe

# 3. 开发运行
cargo run --release -- D:\study.git\testdata\Part1.stp
```

窗口内也可以通过 **文件 → 导入…** 选择文件;最近打开的 10 个文件会记忆在 **文件** 菜单里。

**无界面自检**(导入/细分/测量/离屏渲染一条龙,用于验证环境与性能):

```bash
cargo run --release -- --selftest 模型.stp [--png 输出.png] [--section]
```

## 支持格式

与 SolidCraft 同一套导入清单:

- B-rep:STEP(`.stp` / `.step`)、IGES(`.igs` / `.iges`)、FreeCAD(`.fcstd`)、Rhino(`.3dm`)、CATIA V5(`.CATPart`)
- 网格:STL(`.stl`)、3MF(`.3mf`)、OBJ(`.obj`)、glTF(`.glb` / `.gltf`,嵌入缓冲)
- 不支持:NX、Creo、SolidWorks 等其余专有格式(先另存为 STEP)

CATPart 由本程序的转换支路(cadmpeg 清洁室编解码器,Apache-2.0)解码重述为 STEP 后走 STEP 导入链,标准布局的实体几何与拓扑可读;个别存储布局或特征历史可能报部分缺失,以警告形式提示。打开时会在源文件旁生成同名 `.stp` 中间文件(可检视、可复用);旁有非本程序生成的同名文件时不覆盖,以警告说明。

## 功能一览

- 界面与 SolidCraft 同款:自绘标题栏应用栏、页签式 ribbon(视图/检查)、浮动卡片面板、IBM Plex 界面字体、暗/亮/跟随系统主题
- 打开 STEP/IGES/FreeCAD/Rhino(B-rep 装配树、名称、颜色)与 STL/3MF/OBJ/glTF(网格),最近文件记忆
- 模型树浮动卡片逐节点显隐(眼睛图标);测量/构造结果自动列入同卡片的「几何特征」区(眼睛显隐 / × 删除 / 点名字选中)
- 选择:单击选面 / 双击选体,视口与模型树联动高亮
- 断面:方向/深度实时剖切 + 剖面填充线(断面分析浮窗)
- 测量:点-点距离、体-体距离、质量属性(体积/表面积/包围盒/质心)、面信息(面积/法向)、质心标记
- 构造:参考点 / 两点轴 / 边轴 / 圆柱轴 / 坐标系
- 视图:旋转/平移/缩放、适配、标准视角(前视/等轴测)、导航罗盘(点轴尖切换视角)
- 状态提示:工具提示在视口左上角,一次性消息为底部 toast(6 秒消失,无状态栏)
- 中文界面

不做:参数化建模、爆炸视图。

## 开发状态

M0–M5 全部里程碑完成(依赖接线 → 导入渲染 → 选择高亮 → 断面 → 测量 → 视图工具/界面改版),过程与决策记录见 [docs/feasibility.md](docs/feasibility.md)。

## 许可

MIT,见 [LICENSE](LICENSE)。
