# Weavelo 移动端 (Android / iOS / iPadOS) UI & 零遮挡交互设计规范

> [!IMPORTANT]
> **Symphonic Zero-Occlusion Mobile Architecture**：
> 1. **工业级交响夜曲美学 (`Philharmonie Nocturne`)**：与桌面端及 `forge-collab` 100% 统一，采用 **深空曜石黑 (`#080B11`)**、**板岩蓝灰卡片 (`#111622` / `#171F30`)**、**Resonant Cyan (`#38BDF8`)**、**Maestro Gold (`#E5A93C`)** 与 **Harmonic Emerald (`#10B981`)**。
> 2. **画面即绝对主体 (100% Unobstructed Canvas)**：在 **手机拉手机 (Mobile → Mobile)** 与 **手机拉电脑 (Mobile → Desktop)** 场景下，远端画面四角、顶部状态栏/标题栏与底部导航栏/任务栏在沉浸态下 **100% 零遮挡**。
> 3. **可垂直拖拽磁吸边缘微胶囊 (Draggable Magnetic Edge Micro-Pill)**：HUD 自动隐去或点击 `Dock` 收起后，仅在屏幕右边缘保留 `6×44dp` 的半透明磁吸微胶囊（空闲态透明度 `38%`），支持手指上下自由拖动避让任何远端按钮。
> 4. **两段式防误触返回守卫 (Two-Stage Anti-Mispress Back Guard)**：彻底解决手机侧滑返回手势误触远端 App 返回却直接断开远程连接的问题。

---

## 📱 Stitch 最新统一高保真设计稿

### 1. 移动端竖屏极简控制中心 (`MobileConsoleScreen.kt`)

- **Stitch 归档预览**：[`design/screens/stitch-screen-4-mobile-portrait-console.png`](../design/screens/stitch-screen-4-mobile-portrait-console.png)
- **HTML 原型源码**：[`design/html/screen-4-mobile-portrait-console.html`](../design/html/screen-4-mobile-portrait-console.html)

#### 结构与功能模块：
- **状态顶部栏 (Status Header)**：
  - 极简品牌标识 `Weavelo`；
  - 实时在线组网胶囊 `N online` / `Discovering`（带 `6dp` `Harmonic Emerald` 在线指示灯）；
  - 右侧一键扫码按钮 `[Scan QR]` 与组网管理入口 `[Add device]`；
- **本机共享状态卡片 (This Phone Sharing Card)**：
  - 直观控制本机是否允许对端拉取屏幕/摄像头、收发文件及剪贴板同步；
- **发现的设备矩阵卡片 (`HostCard`)**：
  - 采用 `#111622` 板岩表面 + `1dp` `#222D42` 细边框；
  - 清晰展示设备图标、在线绿点、`LAN / P2P Direct / Relay` 路由徽章（直连以 `Maestro Gold` 或 `Resonant Cyan` 标识）以及 `[Open screens]` / `[Send file]` 胶囊动作按钮；
- **底部悬浮磨砂导航坞 (`BottomNavigationBar`)**：
  - 居中圆角胶囊导航栏：`[Devices]` · `[Network]` · `[Settings]`。

---

### 2. 移动端横屏 100% 满屏零遮挡触控视口 (`WorkspaceScreen.kt` & `MobileViewportScreen.kt`)

- **Stitch 归档预览**：[`design/screens/stitch-screen-3-mobile-zero-occlusion-viewport.png`](../design/screens/stitch-screen-3-mobile-zero-occlusion-viewport.png)
- **HTML 原型源码**：[`design/html/screen-3-mobile-zero-occlusion-viewport.html`](../design/html/screen-3-mobile-zero-occlusion-viewport.html)

#### 核心交互与零遮挡机制：
1. **顶部悬浮控制岛 + 一键 `Dock` 收纳**：
   - 展开态显示远端设备名、连接状态、`[Dock]` 按钮、源窗口选择器、刷新与更多操作（发送文件、网络共享文件、剪贴板同步、后台保活设置）；
   - `3.5s` 无操作自动隐去，或点击 `[Dock]` 立即收纳为右侧边缘微胶囊。
2. **可垂直拖拽磁吸边缘微胶囊 (`ZeroOcclusionHudPolicy`)**：
   - 收起态下位于屏幕右侧边缘（默认垂直位置 `32%`，避开顶部关闭按钮与底部导航栏）；
   - 支持垂直拖拽（安全范围 `12%..88%`），点击即刻呼出完整 HUD。
3. **右下角轨道式触控舱 (Orbit Touch Pod) + 可折叠粘滞修饰键栏 (`StickyKeyController`)**：
   - 触控舱集成：`[Reset zoom]`（放大超 `1.05x` 时自动浮现）、`[Multi-touch / Direct touch / Touchpad]`、`[Adjust view]`、`[Keys]` 修饰键开关与 `[Keyboard]` 系统输入法开关；
   - 点击 `[Keys]` 在触控舱上方展开单行胶囊修饰键栏：`Esc`, `Ctrl`, `Alt`, `Win`, `Shift`, `Tab`, `F5`, `F11`, `▲`, `▼`, `◄`, `►`；
   - 修饰键（`Ctrl`/`Alt`/`Win`/`Shift`）支持单指轻触锁定（Sticky Latch），锁定态以 **Maestro Gold (`#E5A93C`)** 高亮，并在 `[Keys (N)]` 按钮上实时提示已锁定键数。
4. **两段式防误触返回守卫 (`ZeroOcclusionHudPolicy.evaluateBackPress`)**：
   - **Stage 1（收起浮层）**：若源选择器、菜单、设置弹窗或顶部 HUD 处于展开态，系统返回手势仅关闭当前浮层，进入 100% 满屏零遮挡视口；
   - **Stage 2（二次确认断连）**：若已处于 100% 零遮挡视口，首次侧滑返回不会断开连接，而是唤醒 HUD 并在底部弹出 `Maestro Gold` 提示胶囊（`"Swipe Back again within 3s to disconnect · or tap Back in top bar"`），3 秒内再次返回才执行断开。
