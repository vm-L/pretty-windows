# Pretty Windows 项目开发指南（AI & Engineering Guide）

## 1. 项目定位

`Pretty Windows` 是面向 **Windows 11** 的轻量级系统美化与桌面交互工具集（基于 Rust + 原生 Win32 / Direct2D / DirectWrite / WinRT）。
应用以**系统托盘驻留宿主**作为唯一入口，按需托管与开关各独立美化模块（当前包含任务栏透明媒体控制挂件 `mediabar`、系统监控挂件 `monitorbar` 与 Win11 任务栏风格控制 `taskbar` + XAML 诊断组件 `tap`）。

---

## 2. 目录与模块架构

```text
assets/
└── PrettyWindows.ico  # 多分辨率 App 与系统托盘图标（16..256px 黑白撞色液态玻璃四格视窗）
build.rs               # 零依赖 .ico -> Win32 .res 编译脚本，为 PrettyWindows.exe 嵌入主图标资源
tap/                   # 独立工作区 cdylib（PrettyWindowsTap.dll），经 XAML Diagnostics 安全调整 Win11 原生底色透明度
└── src/lib.rs         # 跨平台视觉树匹配索引单测 + GIT/FTM COM / UI 线程 Dispatcher 调色与 Pipe IPC
src/
├── main.rs            # 应用入口：Per-Monitor DPI V2、COM STA、单实例 Mutex，启动 tray::run()
├── color.rs           # 跨平台色彩与像素算法（RGB/HSL、主题强调色校准、封面主色提取、盒式高斯模糊）
├── system.rs          # [Windows] 共享系统能力（任务栏深浅主题、系统强调色、开机自启注册表）
├── ui/
│   ├── mod.rs         # 跨平台 UI 设计 Token、RectF、弹簧与缓动物理、液态丝绸光场算法
│   └── glass.rs       # [Windows] 共享 Liquid Glass Direct2D 渲染原语（含描边、滑块、开关、设置图标、抓手指针、背景采样）与 LayeredSurface
├── tray/
│   ├── mod.rs         # 托盘宿主（隐藏顶层窗口、Shell_NotifyIconW、内嵌多分辨率 App/Tray 图标加载、模块生命周期管理）
│   ├── layout.rs      # 跨平台托盘级联菜单几何布局、四象限鼠标锚点定位、开合动画矩形与单元测试
│   └── menu.rs        # [Windows] 沿用 Liquid Glass 融合轮廓的托盘级联菜单（媒体控制 / 系统监控 / 任务栏风格 / 自启 / 退出）
├── taskbar/           # 独立的 Win11 任务栏外观管理模块（仅依赖 crate::color / crate::ui / crate::system）
│   ├── mod.rs         # TaskbarController 生命周期、崩溃熔断回退与退出恢复控制
│   ├── config.rs      # TaskbarStyle 五档风格枚举与 Taskbar.toml 持久化配置
│   ├── swca.rs        # 多显示器任务栏枚举与后置背景窗 SetWindowCompositionAttribute 策略映射
│   ├── tap_host.rs    # 异步后台 InitializeXamlDiagnosticsEx 注入（Shadow Copy 防锁）、冲突检测、崩溃熔断与 Pipe IPC
│   └── liquid_glass.rs# 全宽后置背景窗管理（模糊/亚克力与 Liquid Glass 双层 D2D 丝绸渲染，禁用 Win11 圆角裁切）
├── monitorbar/        # 独立的任务栏系统监控挂件模块（仅依赖 crate::color / crate::ui / crate::system）
│   ├── mod.rs         # 模块导出、MonitorBarInstance 生命周期封装与 WM_APP +40..+48 常量
│   ├── app.rs         # MonitorBar 主窗口状态机、拖拽/左键行为/右键菜单调度
│   ├── config.rs      # MonitorBar.toml 持久化配置（enabled / locked / offset_x / left_click / show_*）
│   ├── layout.rs      # 3 列紧凑几何布局、隐藏列收缩与垂直居中单元测试
│   ├── metrics.rs     # 1 秒后台性能采样（网速/CPU/内存/显卡/硬盘）与格式化/聚合单元测试
│   ├── wireless.rs    # WinRT/Win32 WiFi 与蓝牙状态开关、枚举与配对/配置文件控制
│   ├── brightness.rs  # 多显示器异步独立亮度控制（DDC/CI 与内置屏背光为主，回退软件 Gamma 并退出恢复）
│   ├── actions.rs     # 左键动作执行（任务管理器/Win+D/设置/自定义程序）与原生文件选择器
│   ├── popup.rs       # 融合轮廓 Liquid Glass 右键级联菜单（WiFi/蓝牙/亮度/左键/数据/设置）
│   ├── render.rs      # MonitorBar Direct2D 渲染器（静默透明底、悬停胶囊与 >=85% 高负载提醒色）
│   └── taskbar.rs     # Win11 TrayNotifyWnd 左侧右锚定、全屏/自动隐藏守卫与 Z-order 维护
└── mediabar/          # 独立的任务栏媒体控制器模块（仅依赖 crate::color / crate::ui / crate::system）
    ├── mod.rs         # 模块导出与 MediaBarInstance 生命周期封装
    ├── app.rs         # MediaBar 主窗口状态机、鼠标交互、内联音量 HUD（由宿主托管，不独占消息循环）
    ├── config.rs      # MediaBar.toml 持久化配置（enabled / locked / auto_color / offset_x）
    ├── popup.rs       # MediaBar 级联菜单（播放源 / 混音器 / 音频设备 / 设置）
    ├── mini_player.rs # MediaBar 迷你播放浮窗（大封面、进度条点击/拖拽跳转与快捷播放控制）
    ├── timeline.rs    # 跨平台 SMTC 播放进度插值、时间格式化与进度条拖拽定位算法
    ├── render.rs      # MediaBar 任务栏挂件渲染器与抓取光标生成
    ├── layout.rs      # MediaBar 挂件几何与命中测试
    ├── taskbar.rs     # Win11 任务栏定位、全屏/自动隐藏守卫、Z-order 维护
    ├── smtc.rs        # WinRT SMTC 会话监听与封面解码
    ├── audio.rs       # 进程隔离 WASAPI 音频频谱捕获
    ├── audio_devices.rs # 播放/录音默认设备枚举与切换
    ├── volume.rs      # 主音量与每应用会话音量控制
    ├── arbitration.rs # 多媒体会话仲裁策略
    ├── dsp.rs         # 频谱 FFT/对数分频分析
    ├── marquee.rs     # 跑马灯文本滚动状态机
    └── process_util.rs# 进程树查询与播放器窗口激活
```

### 依赖边界规则
- **严格单向无环依赖**：`color` → `ui` / `system` → 功能模块（`mediabar`、`monitorbar`、`taskbar`）→ `tray` → `main`（`tap/` 保持零反向依赖的独立工作区成员）。
- 独立功能模块（`mediabar`、`monitorbar`、`taskbar`）**不得依赖 `tray`**，也不得拥有独立消息循环或调用 `PostQuitMessage(0)`；模块内“关闭”动作通过宿主消息异步通知 `tray` 销毁实例并更新配置。
- `tray` 仅通过功能模块公开的控制器/配置接口协调启停，不侵入模块内部渲染与业务逻辑。

---

## 3. 美术风格与 UI 开发规范（强制阅读）

**所有界面、菜单、浮窗、图标与动效开发前，必须先阅读并严格遵循**：
- [docs/design-system.md](docs/design-system.md)（Windows 11 Liquid Glass 设计系统与定量参数规范）

核心约束摘要：
1. 任何新增浮窗必须复用 `crate::ui` 与 `crate::ui::glass` 的玻璃材质堆栈（3 层柔影 + 桌面模糊折射 + 中性玻璃底 + 液态丝绸光带 + 对角扫光 + 渐变高光描边）与 `Spring1D` 物理动效。
2. 禁止在 UI 中直接硬编码脱离深浅色适配的颜色；强调色须经 `color::adjust_accent_for_theme` 校准，危险/关闭项须使用语义危险色。
3. 菜单与托盘交互文字须保持简洁直观、面向用户，不暴露内部技术实现细节。

---

## 4. 构建与验证命令

开发主机为 Linux（WSL），目标平台为 `x86_64-pc-windows-msvc`。所有代码修改完成后需依次验证：

```bash
# 1. 运行跨平台核心逻辑与布局单元测试（color / ui / tray::layout / taskbar / tap 等）
cargo test --workspace

# 2. 对全部 Windows 原生模块进行交叉编译类型检查（须 0 error, 0 warning）
cargo check --workspace --target x86_64-pc-windows-msvc

# 3. 检查 Windows 目标下的测试代码编译（含 #[cfg(windows)] 单测）
cargo check --workspace --tests --target x86_64-pc-windows-msvc
```

### 新增代码注意点
- 新增非 Win32 逻辑（布局计算、配置解析、物理曲线、色彩/DSP 算法）应保持跨平台可编译并附带单元测试，以便在 Linux 环境通过 `cargo test` 即时验证。
- 所有 Win32 窗口消息常量（`WM_APP + N`）需在模块内明确命名与分段，避免宿主/菜单/子模块之间产生消息编号冲突：
  - `mediabar`：`WM_APP + 1` .. `WM_APP + 10`（当前已占用 `+1`..`+10`）
  - `tray`：`WM_APP + 20` .. `WM_APP + 29`（当前已占用 `+20`..`+28`）
  - `monitorbar`：`WM_APP + 40` .. `WM_APP + 49`（当前已占用 `+40`..`+49`）
