# 设备验收与接续记录 · 2026-11-05

## 已交付

四台电脑已从 `05f563b` 正式打包并兼容升级为 `20261105.121`；两部原签名手机已升级 Android 80（`7b615ba`）。六台设备均使用当前 viewport 协议。桌面三平台与 Android 正式包均已发布到 `weavelo.hackerlife.fun:8443/download`，公开下载字节已核对。Mac Studio 与 MacBook 保留各自原有签名；没有重置系统权限或替换证书。

- 来源选择器：固定列、图标与左对齐，同应用分组缩进，名称筛选及稳定排序；桌面实际签名程序与两部手机界面已检查。目录没有可靠父窗口关系，未伪造进程树。预览仍按 [选择器方案](../plans/SOURCE-PICKER-POLISH-20261105.md) 等待用户选择。
- 文件：四台电脑的全部 **12 个发送方向、12 个 Fetch 方向**通过，每个文件 262,181 字节。接收端独立读取并计算 SHA；没有用发送端完成通知代替接收内容验证。Fetch 使用实际完成记录的 `shared.received_path`。
- Mac 原生 CUA：Studio 接收 Book 的专用应用窗口，135 个解码帧、零错误；原生观察、连续快照、缩略区域、点击、按键、滚动通过。Book 独立记录恰好一次点击、一次 a、一次滚动及释放；中心落点误差小于万分之一。旧引用返回 `stale_snapshot_ref`；查看端转到自身管理窗口后，等待中的后续 z 未执行。
- Android：努比亚 NX741J、红米 2409BRN2CC 都已实际显示 Studio 的独立应用窗口；打开设备不会先拉桌面，串流横屏、HUD 收起，结束后回到竖屏设备页。两部手机安装后的 base.apk 独立 SHA 与正式 APK 一致。
- 努比亚输入：直接点击准确触发一次；临时百度输入法提交的 `NU80ABC` 准确出现一次。测试后恢复微信输入法，并独立核对恢复结果。

上述文件证据位于任务目录的 `matrix121-all-desktop-summary.json` 和各端 `matrix121-*` 记录。CUA 证据为 `cua121current-real-book-window-proof.json`、`cua121cancel-real-book-window-proof.json` 及 Book 的原生接收事件记录。两部手机图片为 `android80-{nubia,redmi}-studio-owned-window-actual.png`。发布证据为 `native121-nas-verified.json` 与 Book 的 `android80-nas-publication.log`。

## 软件画面延迟测试

[VisualClockFixture.swift](../../scripts/acceptance/VisualClockFixture.swift) 在 Studio 显示 60 Hz 时钟和冗余像素标记；Book 拉取该窗口，Studio 再拉取 Book 的这一个观看窗口。已经实际同时出画，返回图片中能识别原时钟；全程不拉取其它应用。时钟原点和采样端在同一台电脑，避免另建跨机校时逻辑。

[measure_visual_clock.py](../../scripts/acceptance/measure_visual_clock.py) 通过现有原生 CUA 观察读取，使用 [decode_visual_clock.swift](../../scripts/acceptance/decode_visual_clock.swift) 解码标记。编译两个 Swift 工具后，为 Python 显式提供正式程序、解码器、已连接返程窗口的设备／来源／连接标识及输出目录；默认采样八次，当前 PNG 原位替换，仅保留小型样本记录。两条测试订阅须在同一轮建立并在 120 秒租约内完成；旧窗口标识与过期租约不得重用。

第一轮已取得八个样本，但观察请求约耗时 250 ms，所得 P50 区间约 0.5–252.6 ms、P95 区间约 86.3–336.2 ms，不能据此判断 25 ms。当前修复给观察结果增加所持帧的本机 `decoded_at_utc_us`，在导出前投影时间；鉴权、有效期和输入仍使用原有单调时钟。工具将记录从源窗口开始绘制到返程解码纹理的时间，另保留观察请求区间供诊断。新增时间戳的真实正式程序采样待下一次兼容升级后补全。

这是两段媒体路径的软件测量。没有测物理屏幕呈现，不能除以二宣称单程 glass-to-glass；当前链路使用 Relay，不能代替局域网直连验收。

## 仍需接续

- HO5 实际桌面曾处于 KDE 锁屏：真实截图为黑色，即使有解码与提交记录也不算图像通过。需机主解锁后继续实际画面及输入验收，不能绕过锁屏。
- Linux 编码：320×180 小探针首次 41.748 ms，后续 0.743／0.955 ms。首次仍超过用户 20 ms 上限；完整 PipeWire 分辨率和长时间分布尚未达标验收，不用小探针数字代表日常表现。
- 微信输入法：努比亚恢复微信后仍未输入测试文字；此前普通系统搜索也存在一像素键盘现象。百度对照通过不代表微信已修复。
- 红米输入：系统允许的界面操作能选来源并显示画面，但普通 ADB 点击无效，自动化输入进程被系统终止；真实用户触摸和键盘尚未实机确认。未修改安全开关，不能把自动化失败认定为产品输入失败。
- Android 普通新屏幕共享仍需要机主的 MediaProjection 系统确认；已有相机权限不能代替屏幕会话。无障碍权限按真实系统状态记录，不自动批准。
- Linux／Windows 原生 GPU 图像的 CUA 导出及远端真实 AX 语义树尚未接入；当前明确返回不可用，不生成虚构节点。
- 全设备音频、摄像头、通知／控制往返、主动与被动多窗口组合、旋转／IME／缩放下输入，以及长时间稳定性仍须逐项完成。文件矩阵通过不代表这些媒体项目已通过。
