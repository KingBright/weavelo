# Weavelo — Symphonic Zero-Occlusion Cross-Platform Design System (Stitch + GPUI + Jetpack Compose)

> **Design System Asset ID**: `assets/f455cd387ca34755be11132b143e8cce` (`Weavelo Symphonic Zero-Occlusion Studio`)
> **Stitch Project ID**: `10334755827770469383` (`Weavelo — Symphonic Zero-Occlusion Cross-Platform Remote Workspace`)
> **Aesthetic Lineage**: Aligned with `forge-collab` (`Philharmonie Nocturne` & `Swiss Studio Minimalist` isomorphic high-contrast visual architecture)

本目录归档了 Weavelo 全平台（Desktop GPUI: macOS / Windows / Linux；Mobile Jetpack Compose: Android / iOS）的 **Symphonic Zero-Occlusion（交响夜曲 · 零遮挡跨端远程控制）** 设计系统、Stitch 导出高保真原型（HTML & PNG）以及跨平台 4 场景防误触交互矩阵。

---

## 1. 跨平台零遮挡交互矩阵 (4-Scenario Zero-Occlusion Interaction Matrix)

针对 **手机拉手机、电脑拉手机、电脑拉电脑、手机拉电脑** 四种跨端组合中“悬浮控制栏遮挡远端画面或引发边缘手势误触”的痛点，设计并落地了统一的 **Zero-Occlusion HUD Architecture**：

| 跨端场景 | 历史交互痛点 | 全新零遮挡交互设计 (Zero-Occlusion Solution) | 代码落地位置 |
| :--- | :--- | :--- | :--- |
| **电脑拉电脑 (Desktop → Desktop)** | 鼠标移到画面顶部 `0..=16px`（如切换远端浏览器标签页、点按 macOS 顶部菜单栏或右上角关闭按钮、拖拽窗口标题栏）时，悬浮胶囊频繁弹出遮挡目标区域。 | 1. **三段水平停靠与区域过滤 (`ToolbarDock::Left / Center / Right`)**：仅当指针进入当前停靠槽位对应的水平区段时才触发顶部感应，其余 2/3 顶部边缘完全透传给远端。<br>2. **拖拽防弹窗抑制 (`buttons_held`)**：按住鼠标按键拖拽远端窗口或框选时，100% 抑制顶部悬浮胶囊弹出。<br>3. **零遮挡微导轨模式 (`stream_micro_rail`)**：点击胶囊上的 `⌃` 按钮可将控制岛折叠为 `88×6px` 的极细顶部呼吸光条，折叠态下禁用悬停自动弹出，仅点击微导轨才展开。 | [`restored_stream_controls.rs`](../app/src/restored_stream_controls.rs)<br>[`restored_ui.rs`](../app/src/restored_ui.rs) |
| **电脑拉手机 (Desktop → Mobile)** | 电脑鼠标从手机画面顶部下拉通知栏/控制中心时，触发桌面端顶部悬浮工具栏弹窗，打断下拉手势。 | 1. **拖拽态零干扰**：按住鼠标左键从手机画面顶部向下拖动通知栏时，`should_reveal_stream_toolbar(..., buttons_held = true, ...)` 严格返回 `false`。<br>2. **竖屏画布自适应微导轨**：支持将控制岛停靠在画布左/右外侧或折叠为 `6px` 顶部微导轨，不遮挡手机状态栏与灵动岛。 | [`restored_ui.rs`](../app/src/restored_ui.rs)<br>[`restored_stream_controls.rs`](../app/src/restored_stream_controls.rs) |
| **手机拉手机 (Mobile → Mobile)** | 1. 手机侧滑返回手势直接触发 `BackHandler(onBack = disconnect)` 导致远程会话瞬间断开。<br>2. 隐藏 HUD 后固定在右上角的 `36dp` 按钮遮挡被控手机 App 右上角菜单/搜索按钮。 | 1. **两段式防误触返回守卫 (`ZeroOcclusionHudPolicy.evaluateBackPress`)**：当任何弹窗或 HUD 展开时，侧滑返回优先收起浮层；当画面全屏零遮挡时，第一次侧滑返回仅唤醒 HUD 并弹出 `3s` 二次确认胶囊提示，绝不误断连。<br>2. **可垂直拖拽磁吸边缘微胶囊 (Draggable Magnetic Edge Micro-Pill)**：HUD 收起后化身为贴合屏幕右边缘的 `6×44dp` 半透明微胶囊（空闲时透明度降至 `38%`），支持沿屏幕边缘上下自由滑动避让任意 UI，四角与顶底栏 100% 零遮挡。 | [`ZeroOcclusionHudPolicy.kt`](../android/app/src/main/java/fun/hackerlife/weavelo/ui/ZeroOcclusionHudPolicy.kt)<br>[`WorkspaceScreen.kt`](../android/app/src/main/java/fun/hackerlife/weavelo/ui/WorkspaceScreen.kt) |
| **手机拉电脑 (Mobile → Desktop)** | 1. 顶部双层状态栏 + 右下角常驻工具条遮挡远端桌面窗口标题栏与任务栏。<br>2. `WorkspaceScreen` 缺少桌面修饰键（`Ctrl`/`Alt`/`Win`/`Shift`/`Esc`）与触控模式快速切换。 | 1. **轨道式触控舱 (Orbit Touch Pod)**：集成 `[Multi-touch / Touchpad / Adjust view]`、`[Reset zoom]`、`[Keys]` 与 `[Keyboard]`，一键点击顶栏 `Dock` 即可收纳为侧边磁吸微胶囊。<br>2. **可折叠粘滞修饰键栏 (Collapsible Sticky Modifier Bar)**：支持 `Esc`、`Ctrl`、`Alt`、`Win`、`Shift`、`Tab`、`F5`、`F11` 与方向键，激活态以 **Maestro Gold (`#E5A93C`)** 高亮锁定，打字或收起 HUD 时自动让出全部视口。 | [`WorkspaceScreen.kt`](../android/app/src/main/java/fun/hackerlife/weavelo/ui/WorkspaceScreen.kt)<br>[`MobileViewportScreen.kt`](../android/app/src/main/java/fun/hackerlife/weavelo/ui/MobileViewportScreen.kt) |

---

## 2. 视觉色彩与设计 Token (Philharmonie Nocturne & Swiss Studio Minimalist)

与 `forge-collab` 保持同源的高对比度双主题（Dark / Light）色彩架构，所有主文本对比度均满足 WCAG AAA（$\ge 11:1$）：

| Semantic Token | Dark Theme (`Philharmonie Nocturne`) | Light Theme (`Swiss Studio Minimalist`) | 用途说明 |
| :--- | :--- | :--- | :--- |
| `bg_canvas` / `ColorBgPrimary` | `#080B11` | `#F8FAFC` | 沉浸式深空曜石底色 / 瑞士极简浅灰画布 |
| `bg_surface` / `ColorSurfaceCard` | `#111622` | `#FFFFFF` | 设备卡片、侧边抽屉与模态面板主表面 |
| `bg_elevated` / `ColorSurfaceElevated` | `#171F30` | `#F1F5F9` | 悬浮控制岛、键帽胶囊与二级交互容器 |
| `bg_glass` | `rgba(17, 22, 34, 0.84)` | `rgba(255, 255, 255, 0.88)` | 零遮挡毛玻璃 HUD 浮层（24px Backdrop Blur） |
| `text_primary` / `ColorTextPrimary` | `#F3F6FC` | `#0F172A` | 主标题、设备名称与核心状态文字 |
| `text_secondary` / `ColorTextSecondary` | `#94A3B8` | `#334155` | 副标题、分辨率与连接路径标签 |
| `accent_cyan` / `ColorAccentCyan` | `#38BDF8` | `#0284C7` | **Resonant Cyan**：主交互焦点、活跃流、P2P/LAN 信道 |
| `accent_gold` / `ColorAccentGold` | `#E5A93C` | `#D97706` | **Maestro Signal Gold**：远程键鼠控制接管、粘滞修饰键锁定、直连徽章 |
| `accent_emerald` / `ColorAccentEmerald` | `#10B981` | `#059669` | **Harmonic Emerald**：120FPS 硬件解码健康态、设备在线指示 |
| `danger` / `ColorDanger` | `#F43F5E` | `#DC2626` | **Signal Coral**：断开连接、权限告警与异常提示 |

---

## 3. Stitch 高保真设计稿与 HTML 原型归档

| 序号 | 屏幕名称 | 平台与场景 | PNG 预览图 | HTML 源码 |
| :--- | :--- | :--- | :--- | :--- |
| **Screen 1** | **Desktop Mesh Hub & Device Matrix** (`29f3d95a6f4445b5b55f18b27df46c00`) | Desktop (`2560×1440` GPUI 主控台：左侧玻璃设备矩阵 + 源窗口浏览器 + 5 信道遥测) | [stitch-screen-1-desktop-mesh-hub.png](screens/stitch-screen-1-desktop-mesh-hub.png) | [screen-1-desktop-mesh-hub.html](html/screen-1-desktop-mesh-hub.html) |
| **Screen 2** | **Desktop Zero-Occlusion Stream Cockpit** (`b7dc9f357d824265a63ae8684d8f8008`) | Desktop → Desktop & Desktop → Mobile (100% 画布 + 顶部 6px 微导轨 + 三段停靠控制岛 + PiP 手机竖屏流) | [stitch-screen-2-desktop-zero-occlusion-cockpit.png](screens/stitch-screen-2-desktop-zero-occlusion-cockpit.png) | [screen-2-desktop-zero-occlusion-cockpit.html](html/screen-2-desktop-zero-occlusion-cockpit.html) |
| **Screen 3** | **Mobile Zero-Occlusion Touch Viewport** (`68ca6a7bee2243198fad9ba8bc726aef`) | Mobile → Desktop & Mobile → Mobile (横屏满屏 + 右侧磁吸微胶囊 + 轨道式触控舱 + Maestro Gold 粘滞修饰键栏) | [stitch-screen-3-mobile-zero-occlusion-viewport.png](screens/stitch-screen-3-mobile-zero-occlusion-viewport.png) | [screen-3-mobile-zero-occlusion-viewport.html](html/screen-3-mobile-zero-occlusion-viewport.html) |
| **Screen 4** | **Mobile Portrait Mesh Console** (`e9fdf1c840a643a78c10966a62fdc495`) | Mobile Portrait (`Philharmonie Nocturne` 竖屏设备矩阵 + 扫码入网 + 本机共享开关 + 底部毛玻璃导航坞) | [stitch-screen-4-mobile-portrait-console.png](screens/stitch-screen-4-mobile-portrait-console.png) | [screen-4-mobile-portrait-console.html](html/screen-4-mobile-portrait-console.html) |
