export const chinese = {
  "Weavelo - Ultra-Low-Latency Web Console": "Weavelo - 超低延迟网页控制台",
  "Connection": "连接",
  "Weavelo Ready": "Weavelo 已就绪",
  "Connecting…": "正在连接…",
  "Waiting for video…": "正在等待视频…",
  "Weavelo Live": "Weavelo 实时画面",
  "Connection failed": "连接失败",
  "Direct touch mode": "直接触控模式",
  "Toggle virtual shortcut bar": "切换虚拟快捷键栏",
  "Toggle fullscreen": "切换全屏",
  "Hide top controls": "隐藏顶部控制栏",
  "Show top controls": "显示顶部控制栏",
  "Microphone transport is not connected in the web client": "网页客户端尚未连接麦克风传输",
  "Clipboard transport is not connected in the web client": "网页客户端尚未连接剪贴板传输",
  "TOUCH": "触控",
  "KEYS": "按键",
  "FULLSCREEN": "全屏",
  "HIDE": "收起",
  "MIC": "麦克风",
  "CLIP": "剪贴板",
  "DISCONNECT": "断开",
  "Weavelo Console": "Weavelo 控制台",
  "Close connection panel": "关闭连接面板",
  "VIDEO GATEWAY": "视频网关",
  "WebSocket endpoint": "WebSocket 地址",
  "Connect / Reconnect": "连接 / 重新连接",
  "Enter an HEVC Annex B video gateway endpoint. The native Weavelo relay requires a protocol adapter. Discovery, audio and clipboard are not connected in this web client.": "请输入 HEVC Annex B 视频网关地址。原生 Weavelo 中继需要协议适配器。此网页客户端尚未连接设备发现、音频和剪贴板功能。",
  "MEASURED NETWORK ACTIVITY": "实测网络活动",
  "Received binary data": "已接收二进制数据",
  "Realtime Audio (Opus)": "实时音频（Opus）",
  "Unavailable": "不可用",
  "Interactive Control": "交互控制",
  "Reliable File Transfer": "可靠文件传输",
  "Connected. Waiting for a decodable video frame…": "已连接，正在等待可解码的视频帧…",
  "Live video received. Frame rate is measured from rendered frames.": "已收到实时视频。帧率根据已渲染的帧测量。",
  "Latency unavailable": "延迟不可用",
  "Connecting to the video gateway…": "正在连接视频网关…",
  "Disconnected. Enter a gateway endpoint to reconnect.": "连接已断开。请输入网关地址以重新连接。",
  "Interface language": "界面语言",
  "Web Audio is unavailable in this browser.": "此浏览器无法使用 Web Audio。",
  "This browser does not support WebCodecs. Use a supported browser over HTTPS or localhost.": "此浏览器不支持 WebCodecs。请使用支持的浏览器，通过 HTTPS 或 localhost 访问。",
  "Canvas rendering is unavailable.": "Canvas 渲染不可用。",
  "HEVC decoding is unavailable in this browser.": "此浏览器无法进行 HEVC 解码。",
  "Enter a ws:// or wss:// endpoint.": "请输入 ws:// 或 wss:// 地址。",
  "Connection timed out.": "连接超时。",
  "No decodable video received. This client requires an HEVC Annex B WebSocket gateway; the native relay needs a protocol adapter.": "未收到可解码的视频。此客户端需要 HEVC Annex B WebSocket 网关；原生中继需要协议适配器。",
  "WebSocket connection failed. Check the endpoint and its TLS settings.": "WebSocket 连接失败。请检查地址和 TLS 设置。",
  "The remote connection closed. Reconnect to continue.": "远端连接已关闭。请重新连接以继续。"
};
export function resolveLanguage(saved, system = 'en') {
  return saved === 'en' || saved === 'zh-CN' ? saved : /^zh(?:[-_]|$)/i.test(system) ? 'zh-CN' : 'en';
}
export function translate(language, key) { return language === 'zh-CN' ? chinese[key] ?? key : key; }
export function applyLanguage(root, language) {
  root.documentElement.lang = language;
  for (const node of root.querySelectorAll('[data-i18n]')) node.textContent = translate(language, node.dataset.i18n);
  for (const attr of ['title', 'aria-label']) {
    for (const node of root.querySelectorAll(`[data-i18n-${attr}]`)) node.setAttribute(attr, translate(language, node.getAttribute(`data-i18n-${attr}`)));
  }
}
