# Windows 公共发布 helper 最小适配评审

日期：2026-10-25。项目基线：`98999e763337b2cdb0180db1a0f6010b57c22a9d`。
状态：**方案与测试设计；未实施、未获原生验收、不是发布批准。**

## 1. 负责人和本轮边界

公共代码位于 `~/.codex/skills/rust-project-lifecycle/scripts/`，活动检查来自
`~/.codex/skills/clean-mac-storage/scripts/managed_release_retention.py`。
它们不是本项目 Git 文件；本轮没有修改、安装、调用恢复或更新这些 helper。
相同 OS 用户所有权不能证明某个工程线程拥有维护权。

已检查当前可见任务列表及相关交接线程，没有得到公共 helper 的唯一在任维护者
确认。因此实施前由原工程协调者确认原 Rust lifecycle 维护者，并与存储/清理
adapter 维护者约定唯一写者；本任务只负责 Weavelo 接线与调用契约。
不向其它线程发送指令，不据此接管共享文件。

只读、有限调用方核对结果：

| 调用方 | 现有依赖与影响 |
| --- | --- |
| Weavelo `release_artifact_lifecycle.py`、Mac/Linux packager、NAS relay producer | 公共 setup/preflight/admit/publish，原 producer 锁、精确输出组及回执 |
| Octendra `scripts/release_current.py`、`scripts/build_windows_current.py` | 同一公共 helper；后者使用既有 xwin 工具链生产 Windows 产物，不证明 helper 可在 Windows 原生运行 |
| OmniAgent `scripts/publish_hub_current.py` | 导入公共 module，调用 `validate_producer`、目录签名和原 publish；不能改变目录交换或 API 返回值 |
| Forge / Forge Collab | 本轮限定的 `scripts/` 搜索未找到上述直接调用；不据此声明其它入口不存在 |

这不是全 workspace 扫描或穷尽消费者清单。公共维护者仍须核对其已登记调用方。

## 2. 必须保持的事务契约

依据原 `rust_release.py` 的 `admit`、`validate_producer`、`publish`、
`preflight`、`finish_update`，以及项目现有 wrapper：

1. 继续原 authority、release binding、registry、`current_components` 和
   `current_update`；不引入 Windows 独立 ledger、成品代际或源码/target 副本。
2. producer 是长生命周期的本进程或已验证祖先。仅 PID 或锁文件存在均不足。
   `validate_producer` 为只读匹配：component、默认文件组、存活身份、building
   阶段及精确预留必须一致，不能替 child 新建 admission。
3. admit 返回每个默认文件的同目录 `.<basename>.pending` 和已登记 aux stage。
   未知 pending、内容漂移、路径重叠、旧 pending_delivery 均保留并拒绝。
4. 原验证回执必须属于该 staged 输出组。publish 重复同一 producer、文件顺序、
   hash、validation kind/file/digest；源构建回执不能替代包验证回执。
5. 先验证旧 current 和全部新输出，再写 prepared journal。带回执的成员最后替换；
   原契约只承诺逐路径原子替换，不承诺多个文件同时可见。
6. registry 的 committed 记录是提交判据。若写入/flush 报错，先读回实际 registry；
   已提交不能按失败回滚。`current_committed=true` 与 `cleanup_status=deferred`
   可以同时成立。
7. 活动生产者不回收。确认生产者退出后，building 恢复保持 baseline，prepared
   恢复旧组，committed 只收尾；cleaning 按原精确清单恢复。身份、字节、活动或
   持久化状态未知时保留 journal/临时物，观察原操作，不重跑 publish。

现有 Windows 缺口不只是 `fcntl` import：还包括 `os.getuid`/mode 假设、
`ps` 生产者与祖先查询、shlock 全局互斥、protocol FD 证明、目录 fsync、
native path/file identity、输出引用检查和 `rename_atomic(exchange=True)`。

## 3. 最小实现分工和平台接口

下面是待原维护者评审的能力接口，不是新增公共命令或已经可用的能力。
先保留 Mac/Linux 分支原行为；Windows 不可通过 import 成功就报告 supported。

| 适配面 | Windows 候选实现及拒绝条件 |
| --- | --- |
| project/global/producer 互斥 | 原 helper 内的平台 lock context 使用固定文件的 `CreateFileW` + `LockFileEx(EXCLUSIVE, FAIL_IMMEDIATELY)`，固定锁序、持有期、退出释放；清理入口也必须使用相同排斥协议。不能另建一个不会被清理器理解的锁来声称已互斥。 |
| producer identity / ancestor | 普通查询 handle + 创建时间 + owner SID + 映像路径，核对祖先快照前后身份；拒绝 PID 重用、跨用户、无法查询或祖先不明。可复用项目现有 native query 经验，不能将 app 的 `.rpbuild` 内存检查当作 producer 锁证明。 |
| 文件/目录与 metadata 身份 | handle 得到的 volume/file ID、owner SID、link/reparse 状态与规范路径；拒绝 junction、路径别名歧义、跨卷、硬链接及权限不足。保持现有 metadata/registry 写入和持久化语义；不能用 Windows chmod/UID 占位值放行。 |
| protocol 锁豁免 | 只允许原 helper 验证本调用实际持有的精确锁；不得接受外部 JSON/PID 自报或宽泛 Python/父进程豁免。继承 handle 不等于继承字节锁访问证明，无法提供等价证明的 scope 不开放。 |
| scoped activity | 按下一节逐 scope 返回 complete/busy/unknown；任何缺项都不得合成 idle。 |
| publication / recovery | 必须按第 5 节另行评审文件替换与 journal 扩展；不得把 ReplaceFileW 填进现有 exchange 函数后宣布完成。 |

Microsoft 的 [LockFileEx 文档](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-lockfileex)
明确区分锁定 handle、继承 handle 和第二次打开，也说明字节锁不阻止映射视图访问；
因此上述互斥不能替代引用检查。[GetProcessTimes](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getprocesstimes)
支持通过普通查询权取得创建时间，用于绑定 PID，而不是申请调试权限。

Windows 阶段首期只处理**普通文件 current 组**，不实现通用目录交换、identity
transition、全项目清理或其它项目部署；这些 unsupported 能力明确拒绝。
已有 POSIX 记录/调用不能被 Windows 字段改写，unsupported platform schema
不得自动迁移。全局 helper owner 必须评审新增平台记录的兼容版本和拒绝策略。

## 4. 输出引用检查的覆盖契约

检查范围是 admission 的精确 current、staged、已登记 aux/本次 retired 文件，
不是用户整个主目录、全系统内存或凭据。观察器仅输出范围内匹配、PID/创建时间、
file ID、错误码与 coverage，不输出全量命令行或无关私人路径。

| scope | 必需证据；不能替代它的证据 |
| --- | --- |
| process / compiler | 前后稳定的进程身份及 release-path 引用；Cargo/rustc 等需项目归属证明。无法区分另项目编译器时为 unknown，不能停止它或假定无关。 |
| exact open files | 对存在文件的 sharing/占用检查、路径/handle/file ID 前后绑定。可用 Restart Manager 的注册资源查询补充正向占用证据；空列表不单独证明所有 scope idle。 |
| mappings / image | 必须包括“file handle 已关但映射仍活”的 native fixture。`LockFileEx`、进程名称或空 open-handle 列表均不能证明该项通过。普通权限无法覆盖时明确 unknown。 |
| launch references | 只读核对现有用户/系统启动项、Startup 目录、服务和计划任务动作中的精确路径引用；command parsing/别名解析不明或必要读取被拒则 unknown。禁止执行配置文本、启动/停止服务或修改启动项。 |
| paths absent | 只能证明当时路径不存在；仍要检查进程命令/启动引用，发布前后重新验证，不能免除替换后的占用检查。 |

[CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew)
的 sharing 检查和 [RmGetList](https://learn.microsoft.com/en-us/windows/win32/api/restartmanager/nf-restartmanager-rmgetlist)
的注册资源使用者查询是候选证据来源，不是本方案已经证明的完整 Windows observer。
不调用 Restart Manager 的 shutdown/restart 功能。原生 coverage 未证明之前仍拒绝发布。

不新增管理员权限、SeDebugPrivilege、驱动、服务、持久认证、系统/执行策略或
ACL 修改。若普通权限不足，先保存精确 scope/API/error 证明并停在 unknown，
不能改成提升整个打包器运行。

## 5. 文件替换是待决的协议扩展

原 `rename_atomic` 只有 Darwin/Linux 的 exclusive rename/exchange。
[ReplaceFileW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew)
可以为旧文件指定同卷备份，但不是两个路径的交换；部分错误会留下不同的文件名
状态，且 WRITE_THROUGH 标志未实现。不能用其返回值替代 journal/hash 读回。

候选最小设计：普通文件发布仍用原 admit/publish；在同一 current_update 中增加
带平台版本的**每成员 native replace 子状态**。旧字节仅暂存在本次已登记 aux
stage 的固定 retired 路径，受同一身份清单保护，不成为长期备份或另一套成品。

| 时点 | 要持久记录/核对的状态 | 中断后的处理 |
| --- | --- | --- |
| 首次发布，current 不存在 | prepared、new hash/file ID；同卷 no-replace native move | 目标碰撞拒绝；只有 baseline/new 组合完全已知才恢复 |
| 更新前 | old/new 身份、current/staged/retired 三个精确路径、replace-intent | 未知/预存 retired 保留并拒绝 |
| ReplaceFileW 后 | 实际三路径身份和 bytes，不依赖成功/失败码推断 | 未 committed 按已知子状态恢复 baseline；不能重试原 API 猜测结果 |
| retired 规范化为原 stage | 记录 intent，再将已确认 old 移到 stage；再次读回 | 保留可恢复子状态；不能漏掉崩溃窗口 |
| registry committed | current 全新、回执匹配，实际 registry committed | 不回滚新 current，只对本次确认的 old/aux 收尾 |
| cleanup deferred | 原清单、当前 bytes、存活/占用证据 | 新数据/外来 holder 均保留，后续只恢复原 journal |

**评审保留：** 原来的 prepared 恢复只理解 current/staged old/new 交换；上述
三路径中间态需要改 `validate_transaction`、publish、recovery 和 cleanup，
不是可放在 Weavelo 内部的普通函数替换。ReplaceFileW 的失败状态可能暂时改变
默认路径可用性，metadata 持久化也尚未证明等价。如果不能满足原可用性/恢复
要求，方案不得准入；不能偷偷放宽保证、用 TxF、指针/generation 目录或删除旧包
绕过。公共维护者须先批准具体状态机和 native 可行性，再谈实现。
不设置忽略 ACL/merge 错误的标志；原权限无法完成操作时拒绝，不扩大权限。

## 6. Weavelo 调用适配

拟定普通文件组：现有
`target/package/windows/desktop/Weavelo-Windows-x64.zip` 及其同目录
`Weavelo-Windows-x64.verification.json`。拟 component 为
`windows-desktop-x64-release`，在原 authority 中评审登记；真实 Windows binding/
current 记录本轮未读取，不能推断为空。已有 ZIP 缺正式 current companion 时不得
自动 adopt、伪造旧回执或覆盖。

`build_windows_current.py --build-only` 保持现有源码/编译证据职责，不成为 publisher。
原 `target/.weavelo-windows-current.lock` 和固定 identity JSON/RS 仍保护编译输入；
后续需给原 Windows build 和 package wrapper 接入同一项目级 native producer
互斥，不能只让 packager 使用新锁而留下并发编译入口。保留旧锁状态；归属不明
不能删除后重建。此协调改动不要求 build-only 具备发布、替换或清理能力。
项目 wrapper 在已验证公共 Windows capability 下持 producer 锁，preflight → admit
→ 同一 packager 写 admitted staged ZIP/回执 → 验证 → publish → 原 preflight 收尾。
PowerShell execution-policy、FFmpeg、GUI/namespace、源身份门禁全部保留。
不复用普通无 admission 的 Remove-Item/Compress-Archive 路径替换 current。

必须一起评审的消费者：`install_windows_independent.py:reviewed` 硬编码
`.../desktop/Weavelo`，`upgrade_windows_desktop.py` 复用它；两者不能继续读取可能
过期的目录。建议让 plan 直接验证固定 ZIP/回执及原构建 pins，apply 再在安装器
原有 owned staging 中提取、完整验证。安装目标、profile、私有权限策略和原生
运行身份验证不变。现有未压缩目录保留，不在此范围删除或认领。
该消费者接线是 ZIP-only 首期的必要配套，未完成时不开放生产发布。

## 7. 本项目 fixture 测试设计（尚未新增执行）

fixture 只证明纯解析、状态机和调用边界，不产生 Windows native passing receipt。

| ID | 离线场景 | 必须断言 |
| --- | --- | --- |
| F01 | helper missing / unsupported capability / incomplete scope | 源构建与 packager 均不启动，已有 current bytes 不变 |
| F02 | admit 返回 component、路径、producer、stage 任一错配 | child validator 拒绝；不创建第二 admission、不回退裸 ZIP |
| F03 | PID 相同但创建时间/SID不同，祖先链断裂 | 原生产者与协议锁豁免均不成立 |
| F04 | reparse、硬链、大小写/短名别名歧义、跨卷 | 在输出变更前拒绝；不能靠 lexical prefix 放行 |
| F05 | 每个 activity scope 分别 busy / denied / partial / absent | coverage 完整才允许 idle；absent 不抹掉 launch/process scope |
| F06 | child 只有继承 handle 或伪造 protocol JSON | 不得据此放行锁豁免 |
| F07 | 正常第一次发布与更新，ZIP/回执换序、hash漂移 | 验证完成后 prepared，回执最后，registry commit 后才报告提交 |
| F08 | 第 5 节每个 native substate 的全部 old/new/absent/foreign 组合 | 仅已知组合恢复；foreign/不明持久化保留，不重试 publish |
| F09 | registry replace 已完成但 flush/返回报错 | 先读回 commit，禁止回滚已记录的新 current |
| F10 | cleanup 前后出现新文件/holder/identity漂移 | current 可提交但 cleanup deferred；不递归扩大删除 |
| F11 | ZIP 与旧未压缩目录不同 | plan/apply 只消费被 pins 绑定的 ZIP，不能安装 stale tree |
| F12 | Restricted/AllSigned/Internet-zone 拒绝 | 不改策略、不 Unblock-File、不通过 Python 重写包规避拒绝 |
| F13 | Mac/Linux 原有 current/目录/identity-transition fixtures | 行为、schema 和返回契约保持；Windows unsupported 不传播到旧分支 |

未来测试位置建议：本项目 `scripts/tests/test_windows_release_adapter.py` 和已有
Windows installer/upgrade fixtures；公共模块测试由公共维护者在原 skill 套件维护。
本轮既有 808 项通过记录仅属于上一阶段接线修复，不是这些尚未实现用例的结果。

## 8. 真实 Windows 准入与评审结论

在普通用户、已授权 Windows 主机的**自有临时目录/合成文件**完成：两个独立
process 的真实锁竞争与退出释放；PID 身份/拒绝；短/Unicode/长路径、reparse、
同卷与跨卷；真实 sharing holder、允许 delete-sharing 的 reader、关闭 fd 后仍存
映射；现有原生 EXE image 引用；临时用户 launch fixture（不注册持久启动项）。
不终止他人进程；如拒绝样本需其它安全边界，只记录已有拒绝，不扩大权限制造样本。

对每个持久 journal 边界由 fixture runner 结束自己启动的 publisher，重新调用
同一公共 preflight，核对 current、staged、retired、registry 和回执全部实际 bytes/
file ID。包括 ReplaceFileW 失败、外来修改、sharing race、committed 后清理延期。
无真实 native 结果的项保持 not_tested/unknown；mock/解析成功不能替代。
进程崩溃恢复不等于断电持久性；后者必须另有真实可行性证据，不能捏造通过。

然后才由项目负责人选择已有验证二进制做一次正式包试验；不重新构建已通过产品，
不自动安装、不发布 NAS、不动用户桌面。先独立核对 ZIP/回执，物理视频/输入/
音频与长时验收继续独立。

本轮评审结论：Weavelo 调用/fixture 契约可以继续细化；**公共 helper 实现暂不
准入**。三个待决项是唯一维护者确认、Windows 文件替换/持久化状态机批准、普通
权限下完整引用覆盖的 native 证明。影响面包含 Octendra 和 OmniAgent，必须由
原公共维护者统一修改/验证/发布，不能由 Weavelo 单方面安装一个分叉 helper。

MacBook 原 workspace 请求 `b1f512c2-4cd9-4ec1-af38-1483ca235a2e` 保持唯一；
本轮未重发，也未等待设备恢复来阻塞以上方案工作。
