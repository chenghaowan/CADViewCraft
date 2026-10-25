# CADViewCraft

> **⚠ 主线已切换(2026-10-25):本项目(自研版)封存,当前主线为精简版 SolidCraft —— `D:\study.git\solidcraft` 仓库的 `viewer` 分支,版本 0.0.1 起。**
> 启动:`D:\study.git\solidcraft	argetelease\solidcraft.exe [模型.stp]`,版本号每次发布在 `solidcraft/Cargo.toml` 手动 +0.0.1。
> 以下为自研版的历史文档,仅作备选。


仿 [Mayo](https://github.com/fougue/mayo) 的 3D CAD 查看器,Rust + egui 编写。打开 STEP/IGES 模型,查看装配结构、做断面剖切、进行测量。

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

窗口内也可以通过 **文件 → 打开…** 选择文件;最近打开的 10 个文件会记忆在 **文件** 菜单里。

**无界面自检**(导入/细分/测量/离屏渲染一条龙,用于验证环境与性能):

```bash
target\release\cadviewcraft.exe --selftest 模型.stp [--png 输出.png] [--section]
```

## 支持格式

- 打开:STEP(`.stp` / `.step`)、IGES(`.igs` / `.iges`)
- 不支持:CATIA(`.CATPart`)等专有格式(先另存为 STEP)

## 功能一览

- 打开 STEP/IGES(装配树、名称、颜色),最近文件记忆
- 模型树逐节点显隐
- 选择:单击选面 / 双击选体,视口与模型树联动高亮
- 断面:方向/深度实时剖切 + 剖面填充线
- 测量:点-点距离、体-体距离、质量属性(体积/表面积/包围盒/质心)、面信息(面积/法向)、质心标记
- 视图:旋转/平移/缩放、适配、标准视角(前视/等轴测)、坐标轴指示
- 中文界面

不做:参数化建模、爆炸视图。

## 开发状态

M0–M5 全部里程碑完成(依赖接线 → 导入渲染 → 选择高亮 → 断面 → 测量 → 视图工具/界面改版),过程与决策记录见 [docs/feasibility.md](docs/feasibility.md)。

## 许可

MIT,见 [LICENSE](LICENSE)。
