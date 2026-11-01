# 摄像头拉取、投送与手机前后摄切换

2026-11-01，用户已回复“按方案继续实现”，确认以下原生采集与协议方案。实现与真机验收仍需逐项记录；日常修复、构建、发布和兼容升级继续自动执行。

## 交互与范围

覆盖四台电脑与两部 Android 手机。所有设备都可观看；具有可用摄像头和真实系统许可的设备可作为来源。手机和电脑均能拉取远端摄像头，也能将本机摄像头主动投送给已配对设备。

现有来源选择器增加 Cameras 分组，与 Entire screens、App windows 并列。摄像头项显示实际名称，手机明确区分前置、后置；系统逻辑相机不按其内部传感器重复列出。接收窗口显示设备与摄像头名称，沿用当前全屏／恢复／最小化、连接进度、失败重试、停止与共享／接收状态。

手机发布界面和远端观看窗口均提供紧凑的前后摄切换按钮；其他可用相机通过同一来源列表选择。摄像头支持与屏幕、应用窗口一起作为独立来源收发。硬件不能同时采集时明确显示占用，不关闭其他来源来伪造并行。

第一版交付摄像头视频，不自动开启麦克风。摄像头来源 supports_input=false，不向它注入桌面键鼠；其余文件、屏幕控制及观看窗口保持独立。

## 已核实的起点

- protocol/src/session.rs 的 CaptureSource 只有 MainDisplay、Display、Window，SESSION_VERSION=5；目前没有摄像头来源。
- 桌面继续接入 host/src/capture_sources.rs、host/src/source_catalog.rs、app/src/restored_source_selection.rs 和既有独立订阅／投送流程。
- Android 已声明 CAMERA，但 ProjectionService 只处理 MediaProjection。remote_client_bridge/src/publisher_bridge.rs 只接收 MainDisplay，单份需求、尺寸和帧通道需要改为按来源持有，不能用摄像头覆盖屏幕身份。
- 本次只读枚举：Studio 未列出摄像头；Book 有 FaceTime 高清相机；cube 有状态 OK 的 AMD Camera Device；HO5 仅列出 OBS Virtual Camera。枚举不等于实拍验收。两部手机分别核实 Camera2 实际镜头，不拿另一部手机替代。

## 最小公共扩展

1. 增加 Camera(u64) 来源，以及有界的方向 Front／Back／External／Unspecified、切换组和旋转／镜像元信息。ID 由本地运行时注册表绑定真实原生设备，不使用列表序号或来自客户端的设备路径。设备拔出、撤权或运行时重建使旧绑定失效，不能将旧订阅自动转给另一相机。
2. 沿用 ListSources、Subscribe、Configure、Unsubscribe、DeliveryOffer、来源修订、鉴权和每连接最多 8 个订阅。统一升级现行会话协议到版本 6，不保留旧协议分支；来源选择、合法性校验、生命周期与观看组件共用 Rust 逻辑。
3. Android 发布桥接按来源持有采集需求、尺寸、压缩帧、错误与订阅。屏幕和摄像头各自持有原生资源，停止一个来源只释放对应资源。抽取屏幕／相机共用的编码需求和输出逻辑，保留原生适配器。
4. 同一来源、相同编码参数的观看者共享采集与编码，再扇出压缩帧。不同参数按真实硬件能力处理；资源不足返回明确的 busy／unsupported，不静默改用高开销软件管线。

## 切换前后摄

切换选择另一个真实 Camera 来源。普通拉取沿用换源：结束对应旧订阅、刷新源修订与编码配置、请求关键帧，继续使用同一本地观看窗口。迟到帧不能混入新来源。

增加有界的已认证 RequestCameraSwitch 请求，绑定当前订阅、预期源修订、目标相机及投送请求 ID（普通拉取为空），并返回明确结果。来源端重新校验共享许可、切换组及请求归属。主动投送的来源集合仍不可原地修改：切摄通过新的投送意图／ticket 替换对应旧来源，不扩大旧 ticket 的权限；普通拉取以新订阅完成。

换摄只影响发起请求的观看关系，不隐式切换其他观看者。手机无法同时打开两颗摄像头且旧相机还有其他观看者时，明确返回占用；仅本次观看占用时先释放旧相机，再打开目标相机。失败后显示真实状态、允许重试，不能把旧冻结帧继续标为直播。手机本地切摄遵守相同规则。

## 原生路径与性能

| 平台 | 实现路径 |
| --- | --- |
| macOS | AVFoundation AVCaptureSession／AVCaptureVideoDataOutput，CVPixelBuffer 直接接入既有 VideoToolbox 编码；补齐稳定 bundle 的摄像头用途说明和授权入口。 |
| Windows | Media Foundation 设备枚举与 Source Reader；优先设备支持的 NV12、D3D11/DXGI 原生资源，复用当前 GPU 编码选择与 HEVC 传输。按实际驱动验证缓冲与复制次数，不把 WGC 采屏当作摄像头采集。 |
| Linux | 使用已有 zbus/PipeWire 依赖，Camera portal 的 AccessCamera／OpenPipeWireRemote 提供授权连接，在该连接内枚举、采集；复用现有帧所有权与编码。DMA-BUF 导入以实际编码器／格式支持为准，现有 CPU 帧路径的复制与转色开销单独记录，不称为零拷贝。 |
| Android | Camera2 输出 Surface 接入 MediaCodec 硬件编码，CameraService 持有原生相机与会话；压缩帧进入现有封装与传输，不使用截图、Bitmap 或 JPEG 轮询。 |

不为统一接口将全帧读回 CPU 再转 RGBA；必要的格式转换优先原生/GPU 路径并实测。保留实际色彩矩阵、范围、时间戳和传感器方向，不猜测未提供的色彩信息。

旋转与镜像接入公共可见几何并由观看端 GPU 呈现。默认远端画面不镜像，前置本地预览镜像只影响本地，文字方向应正确；不能只交换宽高或仅设置观看端未使用的编码方向标签。几何改变需刷新来源修订；影响编码尺寸时重建对应编码配置。

队列保持有界，合并过期原始／已解码画面，不丢弃仍被依赖的压缩参考帧。实测首次可见耗时、切摄黑屏时长、有效 FPS、P95/P99 帧间隔、CPU/GPU、复制次数、内存、码率和手机功耗／温度。

## 权限与启动

统一“共享与权限”增加“摄像头”一项，只显示名称、真实状态和一个操作。枚举目录不启动相机。首次启用在本机发起真实系统许可，共享范围沿用当前配对网络；已有许可且平台允许启动时，远端订阅按需采集，最后一个观看者结束后释放相机。撤销共享及时结束相应来源并更新两端状态。

Android 摄像头不使用 MediaProjection，不需要屏幕投屏的逐次确认框；仍需 CAMERA 运行时许可、FOREGROUND_SERVICE_CAMERA 和 camera 前台服务类型。应用前台启动合法的相机服务后，可按系统允许持续共享；纯后台冷启动受系统限制时显示“需要在手机启用”与返回通知，不将一次相机许可承诺为永久任意后台启动。本次视频不自动申请麦克风或改变屏幕投屏授权。

不修改 TCC、AppOps、无障碍或设备安全策略；真实系统授权由本人完成。此方案不启用此前暂缓的 USB 托管采集。

## 实施与验收

1. 公共来源／切摄／几何扩展，定向验证失效绑定、换源迟到帧和独立释放；Android 与 Book 先完成原生视频闭环。
2. 真机分别验证手机前／后摄到电脑、电脑相机到手机，以及手机、电脑各自发起的主动投送。再接入 cube 与 HO5，所有客户端统一当前协议。
3. 按六台设备实际能力记录有向组合，拉取与投送分别验收。无摄像头设备验证接收；虚拟相机单独记录，不能代替物理相机实拍。
4. 覆盖切摄、旋转／镜像、横竖屏、输入法、全屏／多窗口、拒绝／忙碌／撤权／拔插、断网恢复和持续采集，保存实际呈现与性能证据。列表、编译或解码提交不能代替可见画面。

沿用 main、单 worktree、相关检查及既有发布回归，完成迭代即提交、推送并核对远端；签名、打包、发布和兼容升级自动执行，不新增人工补丁哈希、依赖锁门槛、分支或发布代际。

## 官方依据与关联

[Android 镜头枚举](https://developer.android.com/media/camera/camera2/camera-enumeration)、[Camera2 采集会话](https://developer.android.com/reference/android/hardware/camera2/CameraDevice)、[camera 前台服务](https://developer.android.com/develop/background-work/services/fgs/service-types#camera)、[后台启动限制](https://developer.android.com/develop/background-work/services/fgs/restrictions-bg-start#wiu-restrictions)、[AVCaptureVideoDataOutput](https://developer.apple.com/documentation/avfoundation/avcapturevideodataoutput)、[Media Foundation](https://learn.microsoft.com/en-us/windows/win32/medfound/audio-video-capture-in-media-foundation)、[Camera portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Camera.html)。

关联：[统一共享与权限](UNIFIED-SHARING-PERMISSIONS.md)、[主动投送／画板／教学](ACTIVE-PUSH-WHITEBOARD-TEACHING.md)、[原生远端交互](NATIVE-REMOTE-INTERACTION.md)。
