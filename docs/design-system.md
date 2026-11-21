# Pretty Windows 美术风格与交互设计系统规范（Liquid Glass Design System）

> 本文档从现有 `MediaBar` 与共享渲染引擎代码（`src/color.rs`、`src/ui/mod.rs`、`src/ui/glass.rs`、`src/mediabar/layout.rs`、`src/mediabar/render.rs`、`src/mediabar/popup.rs`、`src/tray/menu.rs`）中提炼而成，作为项目所有 Windows 11 桌面悬浮组件、托盘菜单与后续扩展模块的**唯一视觉与动效标准**。

---

## 1. 设计定位与核心原则

1. **Windows 11 原生克制感 Liquid Glass（液态毛玻璃）**：材质通透、干净、有层次，不堆砌高饱和炫光；任何玻璃面板都以中性黑白基底为主，保留桌面内容折射感。
2. **明暗主题自适应**：实时同步 Windows 11 系统任务栏主题（`HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize\SystemUsesLightTheme`），深浅主题拥有独立标定的不透明度、投影与描边高光。
3. **中性玻璃 + 克制流体微光**：禁止在玻璃基底上叠加生硬的冷蓝色偏色（avoid cold blue cast）；动态色彩仅通过低透明度的域扭曲“液态丝绸光带（Liquid Silk Wisps）”与交互高亮控件表达，保证至少 `45%` 面积为清透负空间。
4. **物理连续性与零焦距抢占**：浮窗与任务栏组件均采用 `WS_EX_LAYERED` + Direct2D + `UpdateLayeredWindow(ULW_ALPHA)` 预乘 alpha 离屏渲染；任务栏挂件始终保持 `WS_EX_NOACTIVATE`，弹出菜单通过 `3ms` 子步二阶弹簧（`Spring1D`）驱动连续形变，且在关闭动画期间穿透鼠标点击（`HTTRANSPARENT`）。
5. **纯矢量几何图标与 DPI 无关尺寸**：所有控件尺寸以 DIP（96 DPI 逻辑像素）定义，乘以 `scale = dpi.max(96) as f32 / 96.0`；所有图标均由 Direct2D 路径/基础图元实时绘制，不使用位图图标资源。

---

## 2. 玻璃材质堆栈（由后至前 5 层渲染管线）

所有 Liquid Glass 浮窗卡片必须严格按以下顺序合成：

### Layer 1：环境投影（3-Pass Soft Ambient Shadows）
- 在主轮廓外侧分层绘制 3 遍纯黑填充轮廓，按比例向外扩展：
  - 扩展半径（DIP）：`[7.5 * s, 4.5 * s, 2.0 * s]`
  - 方向性缩放：水平扩展 `expand * 0.55`、顶部扩展 `expand * 0.45`、底部扩展 `expand * 0.65`（级联菜单或贴边浮窗在接缝/贴底边不向外扩）。
  - 投影透明度：
    - **深色模式**：`[0.035, 0.055, 0.09]`
    - **浅色模式**：`[0.022, 0.036, 0.058]`

### Layer 2：桌面折射模糊底（Blurred Desktop Backdrop）
- 弹出窗口显示瞬间通过 `BitBlt(SRCCOPY)` 截取窗口背后的桌面像素区域。
- **降采样与模糊算法**（见 `color::gaussian_blur_bgra`）：
  1. `4×4` 盒式平均降采样（`dw = (w / 4).max(1)`, `dh = (h / 4).max(1)`）。
  2. 按当前任务栏风格配方 `popup_glass_recipe` 中的 `blur_radius_4x`（`透明: 2` / `默认·毛玻璃: 6` / `液态玻璃: 7` / `亚克力: 8`）执行 3 遍可分离水平/垂直盒式模糊。
- 绘制透明度：`recipe.backdrop_alpha`（`透明: 0.58/0.62`、`液态玻璃: 0.82/0.84`、`毛玻璃: 0.85/0.86`、`默认·亚克力: 0.86–0.90`）。
- **截屏失败回退色**：按当前风格基底色调并叠加兜底不透明度 `(recipe.tint_alpha + 0.28).clamp(0.45, 0.92)`。

### Layer 3：风格联动玻璃基底、散射雾面与细腻纹理（Style-Harmonized Base, Veil & Grain）
- 覆盖在模糊桌面之上，统一由 `crate::system::active_taskbar_surface_style()` + `popup_glass_recipe` 驱动所有菜单（托盘、`MediaBar`、`MonitorBar`），保持与 5 种任务栏风格一致的通透度与质感，并基于菜单背后采样亮度（`rgb_luma`）在高反差背景（如深色主题遇亮白应用页面）上自动柔和补足衬底明度与文字对比度：
  - `默认`：稳重原生玻璃底，无雾面颗粒、无丝绸，温和斜向扫光。
  - `亚克力`：高模糊（`r=8`）+ 稳重磨砂底 + 明显 `128×128` 亚克力细噪点颗粒（`max_alpha 11/9`）。
  - `毛玻璃`：清润中性底 + 自上而下乳白散射雾面渐变 + 精细酸蚀微纹理（`max_alpha 6/5`）。
  - `液态玻璃`：高透柔焦（`r=7`）+ 提亮降闷轻底 + 流动液态丝绸光带 + 高光感对角漫反射。
  - `透明`：最清透低遮蔽底（`blur r=2`），无颗粒、无丝绸，轻柔保底对比。

### Layer 4：液态丝绸光带（Liquid Silk Wisps）
- 仅在 `液态玻璃` 风格下激活（悬浮菜单 `silk_alpha: 1.0 / 0.96`，任务栏表层 `1.0 / 0.96`）；`SILK_W = 64` × `SILK_H = 48`（BGRA 预乘格式），每 `SILK_UPDATE_INTERVAL_MS = 32ms` 更新一次。
- 空间归一化：`scale_len = (220.0 * s).max(1.0)`，以卡片左下锚点 `(origin_left, origin_bottom)` 为参考建立归一化坐标 `(x, y)`。
- **同色系三光带域扭曲场**（`sample_liquid_silk_field`）：
  - 坐标先经过两组正交三角波域扭曲（`wx`, `wy`），再叠加三条不同角度的舒缓流动光带（`ang1 ≈ 0.68 rad`、`ang2 ≈ -0.82 rad`、`ang3 ≈ 0.18 rad`）的锐化波峰（`powi(4)` / `powi(5)` / `powi(6)`，次光带权重 `0.78`、第 3 道柔和偏亮同色系高光光带权重 `0.66`）与呼吸包络；第 3 道光带颜色由前两色中值向纯白柔和提亮派生（深色 `0.42` / 浅色 `0.52`），黑灰桌面下仍严格维持中性珠灰层次。
  - **透明上限与负空间约束**：
    - 峰值透明度上限：**深色 `0.27`**、**浅色 `0.21`**。
    - 衰减公式：`alpha = (peak_a * (raw_t / (0.62 + 0.32 * raw_t))).clamp(0.0, peak_a)`，其中 `raw_t = w_sum.min(1.65)`。
    - 保证视野中至少 `33%` 采样点处于静谧清透状态（`alpha < 0.055`）。
- **双光带配色派生**（`compute_halo_pair`）：
  - 源颜色优先级：活跃播放源封面强调色 → 列表首个有效封面强调色 → 系统强调色（托盘/无媒体模块直接使用系统强调色）。
  - 色彩平滑插值速率：`halo_colors` 追随系数 `dt * 4.5`，`ui_accent` 追随系数 `dt * 6.0`。
  - **低饱和（黑白/灰阶）保护**：当基色在 HSL 空间中 `s < 0.12` 时，保持中性珠灰，饱和度钳制为 `s.min(0.10)`，亮度深色 `(0.76, 0.82)` / 浅色 `(0.66, 0.60)`，绝不强行染色。
  - **有彩色光带**：主光带 `s0 = (s * 0.92).clamp(0.12, 0.72)`、亮度深色 `[0.55, 0.68]` / 浅色 `[0.60, 0.72]`；次光带色相顺时针偏移 `shift = 24.0 * ((s - 0.12) / 0.35).clamp(0.25, 1.0)`、`s1 = (s0 * 0.90).clamp(0.10, 0.66)`、`l1 = (l0 + 0.04).clamp(0.57, 0.75)`。

### Layer 5：对角镜面高光与边缘描边（Specular Sheen & Rim Stroke）
- **对角镜面扫光**：从 `union_bounds` 左上角到右下角的白色线性渐变：
  - **深色模式**：`0.0 → white 0.14`，`0.45 → white 0.02`，`1.0 → white 0.06`
  - **浅色模式**：`0.0 → white 0.26`，`0.45 → white 0.03`，`1.0 → white 0.10`
- **边缘玻璃描边（Rim Stroke）**：在裁剪层之外绘制，按任务栏风格配方克制收束高光强度，并在浅色模式与深色高亮度背景下叠画极细外轮廓界定线（`1.25 * s` 宽 `rgba(0, 0, 0, recipe.rim_dark_sep_alpha)`）防止在白底窗口上融边，随后叠加竖向渐变的柔和白色描边。
- **轮廓形态规范**：
  - **独立悬浮圆角卡片（托盘菜单、`MediaBar` / `MonitorBar` 菜单统一规范）**：四角均为完整圆角 `PANEL_RADIUS_DIP = 13.0`；两个 Bar 的菜单悬浮于任务栏顶边上方 `POPUP_FLOAT_GAP_DIP = 7.0` DIP 留白，保留主卡与二级子卡上下接缝处 `STEP_MENISCUS_DIP = 13.5` 的平滑液态弯月过渡；开启菜单时下方任务栏胶囊保持常规闭合高亮轮廓。

---

## 3. 色彩与语义 Token 规范

### 3.1 强调色主题校准与相近色死区保护（`color::adjust_accent_for_theme` / `color::enforce_ui_accent_deadzone`）
任何来源（专辑封面主色、背景采样色或 Windows `UIColorType::Accent`，默认蓝 `Rgb(0, 120, 212)`）用于 UI 高亮、频谱条、进度填充前，必须经过 HSL 校准；同时所有 UI 交互高亮颜色（三处悬浮菜单的选中胶囊、描边、左侧指示条、滑块、开关、播放中徽章，以及 `MediaBar` 频谱柱与内联音量条）在展示前必须经 `color::enforce_ui_accent_deadzone` 进行静态深浅主题灰区死区校验与外推（**液态丝绸光带保留柔和原生色相，不接入死区外推**）：
- **低饱和 / 灰阶色**（`s < 0.08`）：保持原有色相/低饱和，深色目标亮度 `L = 0.76`，浅色目标亮度 `L = 0.37`。
- **有彩色**：`min_s = s.min(0.20)`：
  - **深色模式**：`S = (s * 1.10).clamp(min_s, 0.84)`，`L = l.clamp(0.58, 0.74)`
  - **浅色模式**：`S = (s * 1.08).clamp(min_s, 0.82)`，`L = l.clamp(0.32, 0.48)`
- **静态深浅主题灰区死区与外推**：
  - **深色模式死区**：感知亮度 `Y < 0.36`，或与中性灰区间 `Y ∈ [0.16, 0.62]` 处于椭圆死区（`y_margin = 0.22`、`chroma_margin = 0.42`）以内。
  - **浅色模式死区**：感知亮度 `Y > 0.60`，或与中性灰区间 `Y ∈ [0.54, 0.96]` 处于椭圆死区（`y_margin = 0.24`、`chroma_margin = 0.42`）以内。
  - **外推规则**：原本鲜明且位于死区外的颜色零改动透传；低饱和中性灰（`s < 0.10 || chroma < 0.06`）保留中性特征（`s <= 0.08`）推至高辨识度珠白（深色 `L >= 0.89`）或深炭灰（浅色 `L <= 0.22`）；其余有彩色严格保留原色相 `H`，迭代微调饱和度与明度至刚好离开死区边界即止。

### 3.2 文本、图标与分隔线 Token

| 语义角色 | 深色模式 (`light_glass = false`) | 浅色模式 (`light_glass = true`) |
| --- | --- | --- |
| **主文本 / 主图标** | `rgba(1.0, 1.0, 1.0, 0.95)` | `rgba(0.07, 0.085, 0.12, 0.92)` |
| **次文本 / 次图标** | `rgba(0.94, 0.94, 0.95, 0.72)` | `rgba(0.20, 0.21, 0.23, 0.68)` |
| **闲置 / 提示占位文本** | `rgba(1.0, 1.0, 1.0, 0.56)` | `rgba(0.10, 0.10, 0.12, 0.56)` |
| **危险 / 关闭 / 退出 / 静音斜杠** | `rgba(1.0, 0.48, 0.48, 0.95)`（斜杠 `g/b = 0.46`） | `rgba(0.78, 0.16, 0.18, 0.94)`（斜杠 `a = 0.92`） |
| **水平分隔线** | `rgba(1.0, 1.0, 1.0, 0.14)`，线宽 `1.0 DIP` | `rgba(0.0, 0.0, 0.0, 0.10)`，线宽 `1.0 DIP` |
| **羽化竖向接缝线** | 白色渐变 `0%→0, 14%→0.10, 86%→0.10, 100%→0` | 黑色渐变 `0%→0, 14%→0.075, 86%→0.075, 100%→0` |

---

## 4. 尺寸、圆角与间距系统（DIP）

### 4.1 浮窗与菜单通用几何 Token
- `SHADOW_PAD_DIP = 14.0`：窗口四周预留给投影的透明安全边距。
- `PANEL_RADIUS_DIP = 13.0`：卡片外轮廓圆角。
- `INNER_PAD_DIP = 8.0`：卡片内边距。
- `ROW_H_DIP = 34.0`：标准单行菜单项高度（主菜单项、设置项、托盘菜单项通用）。
- `ROW_GAP_DIP = 4.0`：相邻菜单项垂直间距。
- `DIVIDER_H_DIP = 9.0`：分组分隔线区域总高（左右各内缩 `6.0 * s`，线宽 `1.0 * s`）。
- 菜单宽度参考：一级窄列表 `148.0 DIP`、托盘/设置菜单 `172.0 DIP`、设备列表 `264.0 DIP`、音量混音器 `276.0 DIP`、媒体源列表 `280.0 DIP`。

### 4.2 任务栏内嵌组件 Token（MediaBar）
- 总宽度 `BAR_WIDTH_DIP = 256.0`，右侧固定交互区 `RIGHT_ZONE_WIDTH_DIP = 78.0`。
- 悬停胶囊背景：左右边距 `2.0 DIP`，上下边距 `3.5 DIP`（随任务栏高度自适应钳制），圆角 `7.5 DIP`。
- 封面缩略图：`32 × 32 DIP`，圆角 `6.0 DIP`，细边框 `1.0 DIP`（深色 `white 0.16` / 浅色 `black 0.12`）。
- 控制按钮：`24 × 24 DIP`，悬停底圆角 `5.5 DIP`。
- 频谱柱：`10` 根胶囊条，单柱宽 `3.6 DIP`，全圆角（`width * 0.5`），高度区间 `[3.5, 22.0] DIP`，垂直线性渐变（顶部提亮 `×1.12`，底部略沉 `×0.84`、`alpha × 0.78`）。

---

## 5. 交互状态与反馈规范

### 5.1 菜单行状态（`draw_row_highlight`）
- 圆角：`8.5 * s`（主目录行可用 `8.0 * s`）。
- **未选中 + 悬停**：
  - 深色：`rgba(1.0, 1.0, 1.0, 0.10)`
  - 浅色：`rgba(0.0, 0.0, 0.0, 0.06)`
- **选中 / 启用态**（如当前播放源、当前默认设备、锁定位置开启、托盘中 MediaBar/开机自启开启）：
  - **背景填充**：`accent` 叠透明度——深色 `0.26`（悬停 `0.34`）、浅色 `0.20`（悬停 `0.26`）。
  - **外框描边**：`1.1 * s` 线宽，`accent` 叠透明度——深色 `0.58`、浅色 `0.48`。
  - **左侧强调色胶囊指示条**：宽 `3.0 * s`，高 `(16.0 * s).min(rect.height() - 10.0 * s)`，全圆角 `1.5 * s`，紧贴行内左侧 `rect.left + 3.0 * s`，`accent` 透明度 `0.98`。

### 5.2 按钮与滑块交互
- **按钮悬停 / 按下底色**：
  - 深色：悬停 `white 0.11`、按下 `white 0.18`
  - 浅色：悬停 `black 0.08`、按下 `black 0.14`
- **按下微缩动效**：图标缩放 `0.88×`，前景透明度降至 `0.72`。
- **音量滑块**（`draw_volume_slider`）：
  - 轨道高 `4.0–4.5 DIP`，全圆角；槽底深色 `white 0.20` / 浅色 `black 0.14`。
  - 填充段：`accent` 透明度 `0.96`（静音时 `0.36`）。
  - 滑块圆钮：常态外半径 `5.0 * s`，悬停/拖拽放大至 `6.2 * s`；外圈暗色光晕 `outer_r + 0.9 * s`（深色 `black 0.26` / 浅色 `black 0.18`）+ 白色球体（`0.99` / 静音 `0.86`）+ 中心强调色圆芯（常态 `0.40 * outer_r`，悬停 `0.48 * outer_r`）。

---

## 6. 排版与矢量图标规范

### 6.1 字体与文本排版
- **字体族**：`Segoe UI Variable Text`
- **区域设置**：`zh-CN`
- **排版属性**：`DWRITE_PARAGRAPH_ALIGNMENT_CENTER`（垂直居中），`DWRITE_WORD_WRAPPING_NO_WRAP`（禁止折行），超长文本启用 `DWRITE_TRIMMING_GRANULARITY_CHARACTER` 字符级省略号裁剪（任务栏媒体标题/歌手例外，采用 12px 边缘渐隐遮罩跑马灯）。
- **字号与字重层级（DIP）**：
  - `13.0 DIP Normal`：托盘菜单、`MediaBar` 与 `MonitorBar` 三处弹出菜单主条目文本（不加粗，保持统一轻盈清晰质感）。
  - `12.5 DIP SemiBold`：任务栏挂件主标题（如媒体主标题）。
  - `12.0 DIP Normal`：任务栏闲置提示（`"未在播放"`）。
  - `11.0 DIP Normal`：次级说明文字、空状态提示、设备副名称及右对齐数值（百分比、`MonitorBar` 菜单指标）。
  - `10.5 DIP SemiBold`：状态徽章（`"播放中"` / `"已暂停"`）。

### 6.2 矢量图标风格
- 全站图标采用 Direct2D PathGeometry 与 `Round` 线帽描边（线宽通常为 `1.2–1.45 * s`），几何紧凑居于 `12×12 DIP` 视觉框内：
  - 菜单左侧图标中心通常位于 `rect.left + 19.0 * s`，文本起始于 `rect.left + 34.0 * s`（当带左侧图标时）。
  - 无图标紧凑菜单文本左内边距 `12.0 * s`，右内边距 `10.0 * s`。

---

## 7. 动效物理规范

### 7.1 二阶弹簧系统（`Spring1D`）
- 子步积分步长固定 `SUB_DT = 0.003s`（`dt` 钳制在 `[0.0, 0.1]`），保证不同刷新率下轨迹完全一致。
- 定时节拍：动画阶段 `POPUP_ANIM_INTERVAL_MS = 10ms`；稳态仅维持丝绸流光 `POPUP_FLOW_INTERVAL_MS = 33ms`。

### 7.2 弹出、错峰与收起参数
- **弹窗展开弹簧**：`OPEN_SPRING_STIFFNESS = 780.0`，`OPEN_SPRING_DAMPING = 41.5`，初始状态 `Spring1D::new(0.14, 2.8)`。
  - 特性：快速从底部向上生发并在约 40–60ms 处达到 `1.02..=1.06` 的轻微液态表面张力波峰后回落收敛（约 145ms 接近稳态，380ms 强制收口）。
- **弹窗主透明度淡入**：`window_master_alpha` 在前 `50ms` 内通过 `ease_out_cubic` 从 `0.24` 升至 `1.0`。
- **行错峰微滑（`row_stagger_dy`）**：底部行最先落位、顶部行略微滞后（每行延迟 `7ms`，单行 `105ms` 三次缓出，初始位移 `(5.5 + order * 1.1) * s`，叠加 `(1.0 - open_pos) * 0.28 * 14.0 * s` 的冠部呼吸感）。
- **级联子菜单形变**：宽弹簧 `(920.0, 46.0)`、垂直弹簧 `(860.0, 44.5)`；连续切换分类时继承当前位置与速度，不归零重来。托盘级联菜单（`美化风格` → `148 DIP` 二级菜单）复用同一宽弹簧与融合几何管线，通过 `CombineWithGeometry(D2D1_COMBINE_MODE_UNION)` 消除接缝内凹角并绘制羽化竖向分隔线，未展开子菜单的透明预留区域通过 `WM_NCHITTEST -> HTTRANSPARENT` 保持点击穿透。
- **弹窗收起（Closing）**：时长 `CLOSE_ANIM_DUR_S = 0.095s`（95ms），采用回缩曲线 `ease_retract(t) = t * (0.45 + 0.55 * t)`，窗口透明度 `(1.0 - progress)^1.25`，内容微落 `5.0 * s * r` 且透明度降 `30%`（当托盘菜单向下展开时向顶部锚边同步镜像回缩 `-5.0 * s * r`）。关闭期间 `WM_NCHITTEST` 返回 `HTTRANSPARENT`，避免遮挡或吞掉用户下一次点击。
- **离散功能项执行后自动收起**：托盘菜单（`MediaBar` / `美化风格` 子项 / `开机自启`）与 MediaBar 级联菜单（播放源选择、播放/录音设备切换、`锁定位置`、`重置位置`）在点击生效后立即触发上述 `95ms` 平滑回缩关闭；连续/多步调节项（如主音量/应用静音、滑块拖拽与滚轮调音）保持菜单开启；`退出`/`关闭` 保持立即隐藏（托盘退出时自动恢复默认任务栏与溢出栏外观）。
- **全宽任务栏、托盘溢出栏、`Alt + Tab` 切换器、任务栏悬停预览浮窗、系统硬件 OSD 胶囊、任务栏/托盘悬浮提示胶囊、现代右键菜单 / JumpList 与快速设置 / 通知中心 / 系统通知联动风格（`默认` / `透明` / `毛玻璃` / `亚克力` / `液态玻璃`）**：系统透明开启时，在所有显示器任务栏、可见系统托盘溢出栏（`TopLevelWindowForOverflowXamlIsland` / `NotifyIconOverflowWindow`，裁切 `13 DIP` 外围阴影留白）、可见 `Alt + Tab` 任务切换器卡片、同源 `Taskbar.FlyoutFrame` 浮窗（单/多窗口悬停缩略图预览、纵向滚动列表与任务栏 `...` 溢出浮窗；于任务栏独立 UI 线程的 `XamlExplorerHostIslandWindow` 中由 TAP 通过 `IUIElement::get_RenderSize` + `TransformToVisual(null)` 实时测算 `Border#HoverFlyoutBackground` 的动态边界，并与 `Alt + Tab` 全工作区宿主做线程与尺寸隔离）、系统硬件 OSD 胶囊（`HWConfirmatorUI` 音量 / 亮度 / 键盘背光 / 飞行模式 / 隐私开关浮窗；由 TAP 清除 `Grid#ConfirmatorMainGrid` 的原生亚克力背景、边框与 `ConfirmatorPageShadow` 阴影并精确测算动态轮廓，保留内部滑块、音量强调色及提示图标交互）、任务栏/系统托盘/托盘溢出栏全部 XAML 悬浮提示胶囊（`ToolTip > ContentPresenter#LayoutRoot`，承载于 `band=1` `Xaml_WindowedPopupClass`；仅在 TAP 验证 `IToolTip::IsOpen == true` 且尺寸吻合时接管以严格隔离右键 `MenuFlyout` 等非提示浮窗，清除 `ContentPresenter` 与父级 `ToolTip` 的原生底色、描边与 `ThemeShadow`），任务栏/应用图标/托盘区现代右键菜单与任务栏 JumpList 浮窗（`explorer.exe` 内的 `MenuFlyoutPresenter` 容器表面，最多支持主菜单与 2 级嵌套子菜单共 3 层并发 `band=1` `Xaml_WindowedPopupClass` 槽位一对一防闪烁接管；以及位于 `ShellExperienceHost.exe` AppContainer、`band=6` `Windows.UI.Core.CoreWindow` 宿主内的 `JumpViewUI.TaskbarJumpListFrame` / `Border#JumpListRestyledAcrylic` 与内嵌 `ControlHostMenuFlyoutPresenter`），以及 **快速设置与媒体控制卡片（`Win + A`：`ShellHost.exe` `ControlCenterWindow` 与传统 `ShellExperienceHost.exe` `ControlCenterRegion` / `MediaTransportControlsRegion`）**、**通知中心与日历（`Win + N`：`ShellExperienceHost.exe` `NotificationCenterGrid` + `CalendarCenterGrid`）** 与 **浮动系统通知 Toast（`ShellExperienceHost.exe` `ToastBackgroundBorder2` / `ToastBackgroundBorder`，与通知中心内部历史记录严格隔离）**（均通过双宿主 TAP 协议 v11、PLM 免疫 AppContainer 反向管道与各自独立熔断器隔离托管，多卡片容器统一清除外层容器底色/边框/阴影及次级背景遮罩如 `FooterGrid`、`BackgroundBlur`、`CalendarHeaderMinimizedOverlay` 等，并保留全部原生按钮、滑块、开关、文字、图标及悬停高亮态；并发支持最多 `8` 张可见系统飞出卡片）正后方叠置原生 `Windows.UI.Composition` 实时 GPU 模糊底层（`CreateHostBackdropBrush` + `GaussianBlur(Hard)` + `Saturation`，禁用 `DWMWA_BORDER_COLOR` 系统灰边）+ Direct2D 材质表层（任务栏全宽；溢出栏、`Alt + Tab`、`FlyoutFrame`、OSD 胶囊、悬浮提示胶囊、现代右键菜单、JumpList、快速设置、媒体卡片、通知中心、日历与浮动通知均采用 `DWMWCP_ROUND` 与 `8 DIP` 圆角裁剪及克制圆角描边，OSD 胶囊与系统飞出卡片外侧保留 `10 DIP`、悬浮提示胶囊外侧保留 `6 DIP` 透明边距按比例绘制仅限于卡片外部环域的 3 遍柔和环境投影以替代原生厚重阴影；在 `透明` 档保留极轻清透玻璃底与细腻圆角边界以保证图标、标题与控件辨识度，卡片淡出或尺寸异常时立刻隐藏防残影；普通无边框全屏应用/视频仍保留完整 `band=1` 背板渲染，仅在 D3D 独占全屏/演示模式或上层外部 `WS_EX_TOPMOST` 置顶窗口覆盖卡片区域时自动隐退背板并将对应 `band=18` OSD 容器、`ToolTip > ContentPresenter#LayoutRoot`、右键菜单、`band=6` JumpList 或 `band=4` 快速设置/通知中心/日历/浮动通知主卡片容器原地切换为随 5 档风格与深浅色校准的半透明内联玻璃底、`8 DIP` 圆角与细腻描边（次级子层始终维持透明防双层叠加），始终保持无阴影透感而不回退为原生不透明灰底，仅在切回 `默认` 档或退出时无损还原原生外观）；同步将 `Alt + Tab` 窗口缩略图卡片标题条（`Border#BackgroundBorder`）随 5 档整体风格与系统深浅色联动为轻透玻璃底 + 细腻玻璃描边并拦截悬停实心灰块与 `FocusVisualSecondaryBrush` 内黑边，切回 `默认` 档无损恢复系统原生外观；系统关闭透明时自动切换纯半透明 `WS_EX_LAYERED` 回退，自动隐藏与全屏应用下任务栏按显示器独立隐退，最大化窗口时保持常驻。

---

## 8. Win32 工程约束清单（新增界面模块必读）

1. **DPI 感知**：主进程启动时启用 `DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2`；窗口矩形与字号缓存键需记录 `dpi`，`WM_DPICHANGED` / `WM_DISPLAYCHANGE` 时更新缓存。
2. **托盘宿主窗口禁止使用 `HWND_MESSAGE`**：仅消息窗口收不到 `TaskbarCreated` 与 `WM_SETTINGCHANGE` 广播，宿主必须创建隐藏顶层窗口（`WS_POPUP` + `WS_EX_TOOLWINDOW`，不调用 `ShowWindow`）。
3. **关闭防抖**：弹窗从开始关闭到重新允许触发的防抖窗口为 `240ms`（避免点击锚点图标关闭后在 `WM_ACTIVATE(WA_INACTIVE)` 与点击事件间反复闪烁）。
4. **独立托盘菜单四象限鼠标定位**：鼠标点击托盘图标触发菜单时以 `GetCursorPos` 为锚点（键盘 `NIN_KEYSELECT` 回退图标矩形），按经典 Win32 右键菜单策略优先向**右上方**展开，右侧剩余空间不足翻向**左上方**，仅上方剩余空间不足时翻向**下方**并同步镜像弹簧/行错峰方向；卡片角点贴近鼠标偏移 `2 DIP`，基于完整显示器边界 `mi.rcMonitor`（允许覆盖任务栏）钳制并保留 `4 DIP` 显示器安全边距。
5. **代码复用**：所有新增 Liquid Glass 窗口与菜单必须直接复用 `crate::ui` 与 `crate::ui::glass`，禁止复制 `LayeredSurface`、桌面截屏模糊、丝绸场、行高亮和玻璃描边代码。
