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
└── src/
    ├── lib.rs         # 工作区根导出与跨平台 / Windows 条件编译模块划分
    ├── tree_match/    # 跨平台视觉树名称/属性匹配索引、Pipe 协议常量与单元测试
    │   ├── mod.rs     # 协议常量、TapHostScope/TargetKind 枚举、VisualTreeIndex 核心增删与统一分发
    │   ├── explorer_rules.rs# Explorer（任务栏/溢出栏/Alt+Tab/预览/OSD/ToolTip/菜单/JumpList/切换器）规则
    │   ├── shell_rules.rs# ShellExperienceHost / ShellHost / StartMenu / SearchHost 视觉树匹配规则
    │   ├── tests_explorer.rs# Explorer 作用域视觉树匹配与隔离单元测试
    │   └── tests_shell.rs# Shell 浮窗、搜索面板与树清理单元测试
    └── win/           # [Windows] COM 诊断回调、无损画刷存储、样式计算与 Named Pipe IPC
        ├── mod.rs     # TAP 全局运行时状态与 UI 线程 Dispatcher 调色主循环
        ├── raw_com.rs # 底层 COM 虚表绑定（IVisualTreeService3 / IXamlDiagnostics）与诊断标志位
        ├── com_objects.rs # 诊断对象包装与视觉树变更/销毁清理钩子
        ├── brush_store.rs # 原生画刷快照备份、无损恢复与属性变更监听
        ├── appearance/# 各目标窗口/浮窗卡片外观策略（透明度/边框/降级样式）与坐标测量
        │   ├── mod.rs # 基础阴影/画刷属性同步、边界测量与外观总分派
        │   ├── alttab.rs# 通用 Border、焦点框与 Alt+Tab 卡片标题条覆盖外观管理
        │   ├── flyouts.rs# 悬停预览、OSD、虚拟桌面条与键盘切换器面板外观管理
        │   ├── menus.rs# ToolTip 与 MenuFlyout 圆角/配色同步及无损还原
        │   └── shell_cards.rs# ShellCardSurface/SubSurface（快捷设置/通知/日历/开始菜单/搜索）外观管理
        └── pipe.rs    # 异步命名管道 IPC 服务与缓冲同步回包
src/
├── main.rs            # 应用入口：Per-Monitor DPI V2、COM STA、单实例 Mutex，启动 tray::run()
├── color.rs           # 跨平台色彩与像素算法（RGB/HSL、主题强调色校准、封面主色提取、盒式高斯模糊）
├── system/            # 共享系统能力、窗口 Z-order 守卫、跨栏防碰撞与单元测试
│   ├── mod.rs         # 跨平台表面风格共享状态、Shell/遮挡分类、水平防碰撞纯函数及单元测试
│   ├── theme.rs       # [Windows] 任务栏深浅主题、透明效果开关、系统强调色与开机自启注册表
│   ├── window_state.rs# [Windows] 窗口 ZBID 探测、全屏与遮挡抑制判定及后置背景锚点计算
│   ├── overlay_guard.rs# [Windows] WinEvent 触发式 Z-order 守卫与瞬态飞出遮挡矩形采集
│   └── bar_registry.rs# [Windows] MediaBar / MonitorBar 共享跨栏坐标注册与重排协调
├── ui/
│   ├── mod.rs         # 跨平台 UI 设计 Token、RectF、弹簧与缓动物理、液态丝绸光场算法
│   └── glass/         # [Windows] 共享 Liquid Glass Direct2D 渲染原语与桌面背景模糊折射
│       ├── mod.rs     # LayeredSurface DIB 容器、D2D RenderTarget/TextFormat/色板工具与统一导出
│       ├── backdrop.rs# 桌面背景采集、瞬态浮窗修补遮挡与盒式高斯模糊
│       ├── surface_fx.rs# 玻璃内景堆栈（噪点/丝绸/扫光）、卡片阴影与渐变高光描边
│       ├── controls.rs# 菜单高亮、分割线、文本测量绘制、滑块、矢量图标、胶囊开关与抓手指针
│       └── contour.rs # 主次菜单融合连续外轮廓几何路径构建与三层统一阴影
├── tray/
│   ├── mod.rs         # 托盘宿主（隐藏顶层窗口、Shell_NotifyIconW、内嵌多分辨率 App/Tray 图标加载、模块生命周期管理）
│   ├── layout.rs      # 跨平台托盘级联菜单几何布局、四象限鼠标锚点定位、开合动画矩形与单元测试
│   └── menu/          # [Windows] 沿用 Liquid Glass 融合轮廓的托盘级联菜单（媒体控制 / 系统监控 / 任务栏风格 / 自启 / 退出）
│       ├── mod.rs     # TrayMenu 公开控制器、内部状态、动画推进与窗口生命周期
│       ├── render.rs  # 融合轮廓几何路径、三层投影与 Direct2D 渲染实现
│       ├── icons.rs   # 托盘菜单一级图标与五档风格缩略图标绘制
│       └── wndproc.rs # Win32 消息处理、悬停反馈与菜单项动作分派
├── taskbar/           # 独立的 Win11 任务栏外观管理模块（仅依赖 crate::color / crate::ui / crate::system）
│   ├── mod.rs         # TaskbarController 生命周期、崩溃熔断回退、各浮窗联动同步与退出恢复控制
│   ├── config.rs      # TaskbarStyle 五档风格枚举与 Taskbar.toml 持久化配置
│   ├── swca.rs        # 多显示器任务栏枚举与后置背景窗 SetWindowCompositionAttribute 策略映射
│   ├── tap_host/      # 异步后台 InitializeXamlDiagnosticsEx 注入（Shadow Copy 防锁）、冲突检测、崩溃熔断与 Pipe IPC
│   │   ├── mod.rs     # 熔断常量、进程退出码分类与 Shadow Copy DLL 清理/管理
│   │   ├── inject.rs  # COM 诊断注入激活、第三方冲突检测与 Pipe 协议编解码
│   │   ├── explorer/  # Explorer 主宿主 TAP 客户端、后台调度与 Pipe IPC
│   │   │   ├── mod.rs # TapClient 公开控制器、状态缓存与请求同步方法
│   │   │   ├── worker.rs# 后台注入维护工作循环与各类浮窗坐标轮询状态机
│   │   │   └── pipe_io.rs# 命名管道连接、异步命令收发与卡片矩形查询封装
│   │   └── shell_flyout.rs# ShellExperienceHost / ShellHost / StartMenu / SearchHost 反向命名管道注入管理
│   └── liquid_glass/  # 全宽及 Shell 浮窗后置背景窗管理（禁用 Win11 圆角裁切、双层 D2D 丝绸与 DirectComposition 渲染）
│       ├── mod.rs     # 多显示器任务栏及 Alt+Tab / 预览 / 菜单 / OSD / ToolTip / 切换器 / 搜索背景槽位管理
│       ├── composition.rs# Win32 DirectComposition 高斯模糊与亚克力染色效果管线构建
│       ├── host_windows.rs# 原生 Shell 宿主窗口枚举与跨监视器可见卡片坐标匹配
│       └── renderer.rs# LayeredSurface 分层窗口双层液态丝绸与高光描边绘制
├── monitorbar/        # 独立的任务栏系统监控挂件模块（仅依赖 crate::color / crate::ui / crate::system）
│   ├── mod.rs         # 模块导出、MonitorBarInstance 生命周期封装与 WM_APP +40..+49 常量
│   ├── app.rs         # MonitorBar 主窗口状态机、拖拽/左键行为/右键菜单调度
│   ├── config.rs      # MonitorBar.toml 持久化配置（enabled / locked / offset_x / left_click / show_*）
│   ├── layout.rs      # 3 列紧凑几何布局、隐藏列收缩与垂直居中单元测试
│   ├── metrics.rs     # 1 秒后台性能采样（网速/CPU/内存/显卡/硬盘）与格式化/聚合单元测试
│   ├── wireless.rs    # WinRT/Win32 WiFi 与蓝牙状态开关、枚举与配对/配置文件控制
│   ├── brightness.rs  # 多显示器异步独立亮度控制（DDC/CI 与内置屏背光为主，回退软件 Gamma 并退出恢复）
│   ├── actions.rs     # 左键动作执行（任务管理器/Win+D/设置/自定义程序）与原生文件选择器
│   ├── popup/         # 融合轮廓 Liquid Glass 右键级联菜单（WiFi/蓝牙/亮度/左键/数据/设置）
│   │   ├── mod.rs     # MonitorPopup 公开控制器、内部状态与窗口创建/销毁管理
│   │   ├── layout.rs  # 主次面板几何排布、滚动裁剪、命中测试与弹簧展开动画状态机
│   │   ├── render.rs  # 主菜单与子菜单内容 Direct2D/DirectWrite 分层渲染及接缝微光处理
│   │   ├── icons.rs   # 箭头、WiFi 信号强度、蓝牙与设置等矢量图标绘制
│   │   └── wndproc.rs # Win32 消息分发、鼠标悬停/拖拽交互与无线/亮度动作执行
│   ├── render.rs      # MonitorBar Direct2D 渲染器（静默透明底、悬停胶囊与 >=85% 高负载提醒色）
│   └── taskbar.rs     # Win11 TrayNotifyWnd 左侧右锚定、全屏/自动隐藏守卫与 Z-order 维护
└── mediabar/          # 独立的任务栏媒体控制器模块（仅依赖 crate::color / crate::ui / crate::system）
    ├── mod.rs         # 模块导出与 MediaBarInstance 生命周期封装
    ├── app.rs         # MediaBar 主窗口状态机、鼠标交互、内联音量 HUD（由宿主托管，不独占消息循环）
    ├── config.rs      # MediaBar.toml 持久化配置（enabled / locked / auto_color / offset_x）
    ├── popup/         # MediaBar 级联菜单（播放源 / 混音器 / 音频设备 / 设置）
    │   ├── mod.rs     # SourcePopup 公开控制器、内部状态、消息常量与封面配色辅助
    │   ├── layout.rs  # 主次菜单尺寸测量、动态弹性展开曲线、命中测试与音量条几何
    │   ├── render.rs  # 融合轮廓主视图分派与一级类别行渲染
    │   ├── submenus.rs# 播放源、音量混音器、音频设备与设置四个子菜单内容绘制
    │   ├── wndproc.rs # Win32 消息循环处理、拖拽调音、滚轮调节与关闭收拢
    │   └── tests.rs   # 封面色调校准、丝绸光场边界与动画曲线单元测试
    ├── mini_player/   # MediaBar 迷你播放浮窗（大封面、进度条点击/拖拽跳转与快捷播放控制）
    │   ├── mod.rs     # MiniPlayerPopup 公开控制器、内部状态管理、音量徽章与进度跳转触发
    │   ├── render.rs  # 大封面缩略图、频谱背景光晕、跑马灯文本与控制按钮 D2D 渲染
    │   └── wndproc.rs # 窗口命中测试、进度条拖拽跳转、滚轮音量与点击动作响应
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
  - `mediabar`：`WM_APP + 1` .. `WM_APP + 11`（当前已占用 `+1`..`+11`）
  - `tray`：`WM_APP + 20` .. `WM_APP + 29`（当前已占用 `+20`..`+28`）
  - `monitorbar`：`WM_APP + 40` .. `WM_APP + 49`（当前已占用 `+40`..`+49`）
