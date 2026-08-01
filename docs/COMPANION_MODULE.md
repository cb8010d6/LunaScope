# Desktop Companion Module

桌面伙伴是 LunaScope 桌面壳的一部分，不属于 Skill 或 MCP 扩展。它运行在同一
Tauri 进程中的独立透明 WebView，通过受限命令读取设置和导入用户本地模型，
并以 best-effort 方式接收任务状态；渲染失败不能改变 Agent 运行结果。

## 支持格式

- Spine 3.8：选择 `.skel`，同目录必须包含 `.atlas` 和纹理文件。
- Live2D Cubism 3/4/5：选择 `.model3.json`，目录中必须包含 `.moc3` 和纹理。
  首次导入时，LunaScope 从 Live2D 官方固定 HTTPS 地址获取
  `live2dcubismcore.min.js`，校验大小与运行时标识后存入本地数据目录；专有
  Cubism Core 不提交到本仓库。

两种格式统一实现 `CompanionRenderer` 的 `init / applyPhase / destroy` 契约。
手动导入路径只读取用户明确选择的本地目录；模型库下载是另一条受限路径，
只允许固定目录元数据中的条目，并且必须由用户显式触发。

## 模型库与 Avatar Studio

- 模型库只提交可审计的目录元数据，不提交角色模型二进制。当前内置目录来自
  Ark-Models 的固定 revision，包含 operators、illustrations 和 enemies 三类
  Spine 3.8 条目；条目显示来源、兼容范围和许可证警告。
- 用户点击下载后，Rust 原生命令按文件大小、总大小和 SHA-256/Git blob SHA-1
  校验，写入 staging 目录并原子安装到本机。已安装模型支持搜索、启用和删除，
  当前启用模型不能删除。
- Avatar Studio 会创建标准 `avatar-pack.json`、预览图、图层目录和状态/motion
  映射，支持导入图层、编辑清单、预览、校验、登记和安装。没有合法 Spine 运行时
  导出时，结果明确保持为 draft；它不会把占位图层描述成自动完成专业 rig。
- 只有通过运行时校验的 Avatar pack 才能安装并启用；模型库和 Avatar Studio 的
  下载、安装、预览路径都受本模块的数据目录和 Tauri capability 限制。

## 数据与安全边界

- 设置：`D:\LunaScopeData\companion\settings.json`
- 模型副本：`D:\LunaScopeData\companion\models`
- Live2D Core：`D:\LunaScopeData\companion\runtime`
- 单文件上限 64 MiB，模型总量上限 256 MiB；Live2D Core 上限 4 MiB。
- Spine 只复制 `.skel`、`.atlas` 和纹理；Live2D 使用递归 allowlist，拒绝符号
  链接、脚本和可执行文件，目录深度最多 8 层。
- Tauri asset protocol 只能读取上述模型与运行时目录。
- 不启动独立 HTTP/MCP 服务，不安装 AI 工具配置，也不捆绑第三方角色资产。
- 目录中的第三方模型条目默认按 `NOASSERTION` 处理；使用前必须查看上游条款，
  本地下载内容不会被提交或自动重新分发。

## 状态映射

| LunaScope activity | Companion phase |
|---|---|
| planning / commentary | `working` |
| tool or command execution | `running` |
| reasoning / analysis / verification | `reviewing` |
| paused / waiting / approval | `waiting` |
| completed | `success` |
| failed | `failed` |

Spine 使用语义动画回退；Live2D 按 motion group 名称进行大小写不敏感匹配，找
不到模型特定动作时交回模型自身的 `Idle` 控制器。所有状态转发失败都被隔离，
不会进入持久化运行时、权限引擎或验收结果的可信边界。
