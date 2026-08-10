# Desktop Companion Module

桌面伙伴是 LunaScope 桌面壳的一部分，不属于 Skill 或 MCP 扩展。它运行在同一
Tauri 进程中的独立透明 WebView，通过受限命令读取设置和导入用户本地模型，
并以 best-effort 方式接收任务状态；渲染失败不能改变 Agent 运行结果。

## 支持格式

- Spine 3.8：选择 `.skel`，同目录必须包含 `.atlas` 和纹理文件。
- Live2D Cubism 3/4/5：选择 `.model3.json`，目录中必须包含 `.moc3` 和纹理。
  首次导入时，LunaScope 从 Live2D 官方固定 HTTPS 地址获取
  `live2dcubismcore.min.js`，按 4 MiB 流式上限和固定 SHA-256 校验后原子写入本地数据目录；专有
  Cubism Core 不提交到本仓库。

两种格式统一实现 `CompanionRenderer` 的 `init / applyPhase / destroy` 契约。
手动导入路径只读取用户明确选择的本地目录；模型库下载是另一条受限路径，
只允许固定目录元数据中的条目。永久安装必须由用户显式触发，Spine 会话预加载
只处理当前页的有界子集并按生命周期清理。

## 模型库与 Avatar Studio

- 模型库只提交可审计的目录元数据，不提交角色模型二进制。当前内置目录来自
  Ark-Models 的固定 revision，包含 operators、illustrations 和 enemies 三类
  Spine 3.8 条目；另包含固定到 Live2D 官方仓库提交的 Rice Glassfield 样例元数据。
  Live2D 样例不会自动预加载，下载前必须显示并确认官方 Free Material License、
  Sample Model Terms 和指定版权声明。
- 模型广场使用真实总数、数字过渡、分类和 18 条分页。由于上游目录不提供角色
  缩略图，当前页 Spine 模型按每批最多四个准备不超过 128 MiB 的临时资源，使用
  单实例 renderer 依次渲染一帧并缓存为卡片封面；完成一批后释放 renderer 并替换
  原始资源缓存。翻页会取消过期请求，单个模型失败不会阻断同页其他模型。退出时
  清理残留，异常退出则下次启动重试清理。
- 用户点击下载后，Rust 原生命令按文件大小、总大小和 SHA-256/Git blob SHA-1
  校验，写入 staging 目录并原子安装到本机。已安装模型支持搜索、启用和删除，
  当前启用模型不能删除。
- Live2D 下载要求用户勾选接受两份固定许可，前端会把模型 ID、固定仓库地址和
  两个许可地址作为确认凭据传给 Rust；凭据缺失或与目录不一致时后端拒绝下载。
  下载完成后还会解析 `.model3.json`，拒绝越界、缺失或不受支持的引用文件。
- Avatar Studio 会创建标准 `avatar-pack.json`、预览图、图层目录和状态/motion
  映射，支持导入图层、编辑清单、预览、校验、登记和安装。没有合法 Spine 运行时
  导出时，结果明确保持为 draft；它不会把占位图层描述成自动完成专业 rig。
- 只有通过运行时校验的 Avatar pack 才能安装并启用；模型库和 Avatar Studio 的
  下载、安装、预览路径都受本模块的数据目录和 Tauri capability 限制。

## 数据与安全边界

- 设置：`<data-root>/companion/settings.json`
- 模型副本：`<data-root>/companion/models`
- 临时预加载：`<data-root>/companion/preload`（关闭或下次启动清理）
- Live2D Core：`<data-root>/companion/runtime`
- 单文件上限 64 MiB，模型总量上限 256 MiB；Live2D Core 上限 4 MiB，并固定
  官方文件 SHA-256；已有缓存不匹配时不会执行。
- Spine 只复制 `.skel`、`.atlas` 和纹理；Live2D 使用递归 allowlist，拒绝符号
  链接、脚本和可执行文件，目录深度最多 8 层。
- Tauri asset protocol 只能读取上述模型、临时预加载与运行时目录。应用命令通过
  `AppManifest` 纳入 capability：主窗口获得业务命令集合，隔离 Companion
  WebView 只获得读取设置、保存显示偏好和回报 renderer 状态三项应用命令；
  Rust 命令仍会二次校验调用窗口 label。
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
