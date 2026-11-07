# Weavelo 跨平台 UI & UX 设计系统总览 (Symphonic Zero-Occlusion Architecture)

本文档归档了 Weavelo 全平台（Desktop GPUI: macOS / Windows / Linux；Mobile Jetpack Compose: Android / iOS）的 **Symphonic Zero-Occlusion（交响夜曲 · 零遮挡跨端远程控制）** 设计规范。
完整 Stitch 原型与 HTML/PNG 归档请参见 [`design/README.md`](../design/README.md)。

---

## 目录索引

1. **桌面端设计规范 (GPUI / macOS / Windows / Linux)**
   - 核心原则：以画面为绝对主体（Screen-First / 100% Canvas）
   - 顶部悬浮动态控制岛（Dynamic Island Capsule）与三段水平停靠槽位（`Left / Center / Right`）
   - 零遮挡顶部微导轨（`88×6px` Micro-Rail）与拖拽防弹窗抑制（Drag-Aware Reveal Guard）
   - 左侧滑出式半透明管理抽屉（Slide-over Glass Drawer）与远程源选择器（Source Browser Modal）
   - 独立多流工作区（Multi-Pane Grid / Tabs / Pop-out Window）与 5 信道并发网络遥测
2. **移动端设计规范 (Android Jetpack Compose / iOS)**
   - 竖屏控制中心 (`MobileConsoleScreen` — `Philharmonie Nocturne` 设备矩阵、扫码配对、本机屏幕/文件/剪贴板共享卡片)
   - 横屏 100% 满屏零遮挡远程视口 (`WorkspaceScreen` & `MobileViewportScreen`)
   - 可垂直拖拽磁吸边缘微胶囊（Draggable Magnetic Edge Micro-Pill）
   - 轨道式触控舱（Orbit Touch Pod）与可折叠粘滞修饰键栏（Collapsible Sticky Modifier Bar）
   - 两段式防误触返回守卫（Two-Stage Anti-Mispress Back Guard）
   - 详见完整移动端文档：[MOBILE_UI_UX_DESIGN.md](MOBILE_UI_UX_DESIGN.md)

---

## 🎨 视觉语言与双主题设计系统 (`Philharmonie Nocturne` & `Swiss Studio Minimalist`)

Weavelo 采用与 `forge-collab` 同源的高对比度工业级极简美学，在暗色模式（`Philharmonie Nocturne`）与亮色模式（`Swiss Studio Minimalist`）下均保证 WCAG AAA 级正文可读性（对比度 $\ge 11:1$），代码定义位于 [`app/src/design_system.rs`](../app/src/design_system.rs) 与 [`MobileDesign.kt`](../android/app/src/main/java/fun/hackerlife/weavelo/ui/MobileDesign.kt)：

| 设计元素 | Dark Theme (`Philharmonie Nocturne`) | Light Theme (`Swiss Studio Minimalist`) | 语义与用途 |
| :--- | :--- | :--- | :--- |
| **主画布 (Canvas Base)** | `#080B11` | `#F8FAFC` | 100% 画面视口底色与主窗口深空黑画布 |
| **卡片表面 (Surface Card)** | `#111622` | `#FFFFFF` | 设备矩阵卡片、侧边抽屉与设置面板 |
| **抬升容器 (Elevated Surface)** | `#171F30` | `#F1F5F9` | 顶部悬浮胶囊、触控舱、虚拟修饰键帽 |
| **毛玻璃浮层 (Glass HUD)** | `rgba(17, 22, 34, 0.84)` + `1px` 微边框 | `rgba(255, 255, 255, 0.88)` + `1px` 微边框 | 悬浮控制岛与磁吸微胶囊 |
| **主品牌色 (Resonant Cyan)** | `#38BDF8` | `#0284C7` | 焦点高亮、当前选中视图、P2P/LAN 信道 |
| **控制指挥色 (Maestro Gold)** | `#E5A93C` | `#D97706` | 远程键鼠控制接管激活、粘滞修饰键锁定 (`Ctrl`/`Alt`/`Shift`/`Win`) |
| **健康在线色 (Harmonic Emerald)** | `#10B981` | `#059669` | 60/120FPS 硬件解码稳定态、设备在线圆点 |
| **警示/断开色 (Signal Coral)** | `#F43F5E` | `#DC2626` | 断开连接、录制权限缺失或传输异常 |
| **排版字体 (Typography)** | UI 标签：`Inter` / `Plus Jakarta Sans`<br>遥测与键位：`JetBrains Mono` | 同左 | 遥测数值、帧率、码率、网络延迟与快捷键统一使用等宽字体防抖动 |

---

## 🛡️ 跨平台 4 场景零遮挡交互规范 (Zero-Occlusion Interaction Contract)

1. **电脑拉电脑 (Desktop → Desktop)**：
   - 顶部控制岛支持 **左 (`⇤`) / 中 (`⇔`) / 右 (`⇥`)** 三段水平停靠切换；仅当鼠标落入当前停靠区段且未按住鼠标按键（`buttons_held == 0`）时才触发 `16px` 顶部边缘呼出，避免点击远端浏览器标签页、macOS 菜单栏或右上角窗口按钮时误弹窗。
   - 支持一键折叠为 **`88×6px` 零遮挡顶部微导轨 (`floating_toolbar_micro_rail`)**；折叠态下完全禁用鼠标悬停自动弹出，仅点击微导轨才重新展开控制岛。
2. **电脑拉手机 (Desktop → Mobile)**：
   - 电脑端按住鼠标左键从被控手机画面顶部下拉通知栏/控制中心时，`should_reveal_stream_toolbar` 自动识别拖拽状态并抑制桌面工具栏弹出。
3. **手机拉手机 (Mobile → Mobile)**：
   - **可垂直拖拽磁吸边缘微胶囊**：HUD 收起后不占用四角与顶底栏，而是化为贴合右边缘的 `6×44dp` 微胶囊（空闲时 `38%` 透明度），可沿右边缘上下拖动避让任意 App 按钮。
   - **两段式防误触返回守卫**：侧滑返回优先关闭当前打开的弹窗或收起 HUD；全屏操控状态下首次侧滑返回仅唤醒 HUD 并展示 `3s` 确认提示条，防止手势导航误退出会话。
4. **手机拉电脑 (Mobile → Desktop)**：
   - **轨道式触控舱 + 可折叠粘滞修饰键栏**：在横屏视口右下角提供紧凑触控舱（切换 `Multi-touch / Touchpad / Adjust view`、`Reset zoom`、`Keys`、`Keyboard`），展开 `Keys` 时浮现单行胶囊修饰键栏（`Esc`, `Ctrl`, `Alt`, `Win`, `Shift`, `Tab`, `F5`, `F11`, `▲`, `▼`, `◄`, `►`），点击顶栏 `Dock` 即可一键收纳至边缘微胶囊。
