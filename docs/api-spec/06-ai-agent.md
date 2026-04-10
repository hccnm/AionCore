# 06 - AI 后端集成

## 概述

管理各类 AI Agent 的内部实现：进程生命周期、Worker 通信协议、API 客户端轮转、技能系统、消息后处理中间件，以及 Bedrock/Gemini/Remote Agent 的独立管理接口。

**源码位置**：`process/agent/`、`process/task/`、`process/worker/`、`process/bridge/bedrockBridge.ts`、`process/bridge/geminiBridge.ts`、`process/bridge/remoteAgentBridge.ts`、`common/api/`

> **与模块 5 的边界**：模块 5 定义了会话与消息的外部 API（REST + WebSocket）以及 `IAgentManager`、`IWorkerTaskManager` 等 trait 接口。本模块聚焦于这些 trait 的**内部实现**——每种 Agent 如何启动、通信、管理进程，以及底层 API 客户端和技能系统的工作机制。

## 架构设计

### 功能分区

```
AI 后端集成
├── Agent 实现层         → 6 种 Agent 类型的具体实现
│   ├── ACP Agent       → CLI 子进程 + ACP 协议（Claude、Qwen、CodeBuddy 等）
│   ├── Gemini Agent    → CLI 子进程 + aioncli-core（自研 TS 库，不重写）
│   ├── Aionrs Agent    → 自研 Rust 库，直接集成为 crate
│   ├── OpenClaw Agent  → CLI 子进程 + WebSocket 网关连接
│   ├── Nanobot Agent   → CLI 子进程（阻塞模式）
│   └── Remote Agent    → WebSocket 远程连接（Rust 重实现协议层）
├── Worker 协议层        → 原 TS 实现中的主进程 ↔ Worker 子进程 IPC 协议（Rust 中不沿用）
├── API 客户端层         → 通用 LLM API 客户端（多密钥轮转、协议转换）
├── 技能系统            → 技能发现、索引、按需加载
├── 消息中间件          → 响应后处理（think 标签清理、Cron 命令检测）
├── Remote Agent 管理    → 远程 Agent 配置 CRUD、连接测试、设备配对
└── 连接测试            → Bedrock 凭证验证、Gemini 订阅状态查询
```

### Rust 迁移策略总览

> **核心定位**：Rust 后端是 **CLI 进程编排器 + 协议桥接器**，不重写各 AI 后端的内部协议。

| Agent 类型 | 原 TS 实现 | Rust 迁移策略 | 说明 |
|-----------|-----------|-------------|------|
| `acp` | CLI 子进程 | **CLI 子进程** | 调用 Claude/Qwen 等 CLI 二进制，通过 stdin/stdout 通信 |
| `gemini` | Fork Worker + `@office-ai/aioncli-core` | **CLI 子进程** | aioncli-core 是自研 TS 库，不重写；Rust 调用其 CLI 二进制 |
| `aionrs` | Fork Worker + Aionrs CLI | **直接集成 Rust crate** | aionrs 本身就是 Rust 实现，作为 crate 引入 |
| `openclaw-gateway` | 进程内 WebSocket | **CLI 子进程** | 调用 OpenClaw CLI 二进制 |
| `nanobot` | 进程内 CLI blocking | **CLI 子进程** | 调用 Nanobot CLI 二进制 |
| `remote` | 进程内 WebSocket | **Rust 重实现** | WebSocket 协议层在 Rust 中重写（复用 OpenClaw 连接协议） |

### Agent 进程模型（原 TS 实现）

```
                    WorkerTaskManager (单例)
                         │
           ┌─────────────┼─────────────────────────────┐
           │             │                             │
    Fork 模式           Fork 模式                  进程内模式
    (子进程)            (子进程)                    (无 fork)
           │             │                             │
   ┌───────┤       ┌─────┤                    ┌────────┼────────┐
   │       │       │     │                    │        │        │
 Gemini  Aionrs   ACP  (注)                OpenClaw  Nanobot  Remote
 Worker  Worker  Agent                     Agent    Agent    Agent
   │       │       │                         │                  │
 Pipe    Pipe   AcpConnection          OpenClaw            OpenClaw
 IPC     IPC   (stdin/stdout)          Gateway             Gateway
                                       Connection          Connection
```

> **注**：ACP Agent 虽然继承 `BaseAgentManager`（ForkTask），但实际通过 `AcpConnection` 连接到独立运行的 CLI 子进程，而非标准的 fork + pipe 通信。

### Agent 类型对照（原 TS 实现）

| Agent 类型 | 进程模型 | 底层连接 | 流式事件通道 | 支持 YOLO | Session 恢复 |
|-----------|---------|---------|-------------|----------|-------------|
| `acp` | CLI 子进程 | AcpConnection (stdin/stdout) | `acpConversation.responseStream` | 是（per backend） | 是（session ID） |
| `gemini` | Fork Worker | Pipe IPC → aioncli-core | `geminiConversation.responseStream` | 是 | 否 |
| `aionrs` | Fork Worker | Pipe IPC → Aionrs CLI | `conversation.responseStream` | 是 | 是（`--resume`） |
| `openclaw-gateway` | 进程内 | OpenClawGatewayConnection (WS) | `openclawConversation.responseStream` + `conversation.responseStream` | 是 | 是（session key） |
| `nanobot` | 进程内 | NanobotAgent (CLI blocking) | `conversation.responseStream` | 否 | 否 |
| `remote` | 进程内 | RemoteAgentCore → OpenClawGatewayConnection (WS) | `conversation.responseStream` | 是 | 是（session key） |

## 内部实现详情

### BaseAgentManager

所有 Agent Manager 的抽象基类，继承 `ForkTask`。

**关键行为**：

| 行为 | 说明 |
|------|------|
| 构造 | 根据 `type + '.js'` 定位 Worker 脚本路径 |
| `sendMessage(data)` | 通过 `postMessagePromise('send.message', data)` 发送到 Worker |
| `stop()` | 通过 `postMessagePromise('stop.stream', {})` 停止流式输出，清空确认列表 |
| `addConfirmation()` | YOLO 模式下自动选择第一个选项（50ms 延迟） |
| `confirm(msgId, callId, data)` | 移除确认项，触发 `emitConfirmationRemove` |
| `ensureYoloMode()` | 基类返回 `false`，子类可覆盖 |
| `lastActivityAt` | 每次 `sendMessage` 时更新，用于空闲超时检测 |

### ACP Agent 实现

ACP 是最复杂的 Agent 类型，支持 20+ 种子后端。

**AcpAgentManager 扩展接口**：

```
AcpAgentManager 额外方法（超出 IAgentManager）：
  getMode() → { mode: string, initialized: boolean }
  setMode(mode) → { success, msg?, data? }
  getModelInfo() → AcpModelInfo | null
  setModel(modelId) → AcpModelInfo | null
  getConfigOptions() → AcpSessionConfigOption[]
  setConfigOption(configId, value) → AcpSessionConfigOption[]
  loadAcpSlashCommands(timeoutMs?) → SlashCommandItem[]
  ensureYoloMode() → 调用 agent.enableYoloMode()
```

**AcpAgent 底层协议**：

| 操作 | 协议命令 | 说明 |
|------|---------|------|
| 启动 | `session/new` 或 `session/load` | 创建/恢复会话 |
| 发送消息 | `sendMessage(data)` | 自动重连断开的连接 |
| 确认工具调用 | `confirmMessage({ confirmKey, callId })` | 传递用户确认结果 |
| 取消生成 | `session/cancel` | 停止 LLM 输出但不终止进程 |
| 终止 | `kill()` → 500ms grace → `super.kill()` | 优雅关闭 |
| 启用 YOLO | 设置 session mode | 自动批准所有工具调用 |

**YOLO 模式映射**：

| ACP 后端 | YOLO mode 值 |
|----------|-------------|
| `claude` | `bypassPermissions` |
| `codebuddy` | `bypassPermissions` |
| `qwen` | `yolo` |
| `iflow` | `yolo` |

**Session 恢复策略**：

| 后端 | 恢复方式 |
|------|---------|
| `codex` | `session/load` |
| `claude` / `codebuddy` | `session/new` + `_meta.claudeCode.options.resume` |
| 其他 | `session/new` + `resumeSessionId` |

**ACP 流式事件类型**：

`request_trace`、`slash_commands_updated`、`thinking`、`error`、`finish`、`user_content`、`agent_status`、`content`、`acp_tool_call`、`plan`、`acp_model_info`、`acp_context_usage`、`start`、`thought`、`system`

### Gemini Agent 实现

通过 Fork Worker 运行，底层依赖自研 TS 库 `@office-ai/aioncli-core`（Rust 不重写此库，通过 CLI 子进程调用）。支持 MCP 服务器集成和技能系统。

**关键行为**：

| 行为 | 说明 |
|------|------|
| Bootstrap | 读取 `gemini.config`，加载 MCP 服务器（合并扩展 + 团队配置），发现内置技能，调用 Worker `start()` |
| MCP 指纹检测 | 每次 `sendMessage()` 检测 MCP 配置变更，若变更则重新 bootstrap Worker |
| 技能加载拦截 | 检测 Agent 输出中的 `[LOAD_SKILL: name]`，加载技能内容并注入为 `[System Response]` |
| 确认处理 | `confirm()` → 存储 `ProceedAlways` 到 `GeminiApprovalStore`，调用 `postMessagePromise(callId, data)` |
| Cron 检测 | `finish` 事件后轮询数据库 3 次（间隔 1s/2s/3s）获取最新助手消息 |

**会话模式**：

| 模式 | 自动批准范围 |
|------|------------|
| `default` | 均需确认 |
| `yolo` | 全部自动批准 |
| `autoEdit` | 自动批准 edit/info 操作，exec/mcp 仍需确认 |

### Aionrs Agent 实现

自研 Rust 库，原 TS 实现通过 Fork Worker 调用 CLI。Rust 重写时**直接集成为 crate**，无需子进程调用。

**关键行为**：

| 行为 | 说明 |
|------|------|
| 启动 | 会话有消息 → `--resume {conversationId}`；否则 → `--session-id {conversationId}` |
| 确认处理 | 存储 `ProceedAlways` 到 `AionrsApprovalStore`，调用 `postMessagePromise(callId, data)` |
| 流式输出 | 监听 `aionrs.message` IPC 事件 |

### OpenClaw Agent 实现

进程内运行，通过 WebSocket 连接到 OpenClaw Gateway。

**关键行为**：

| 行为 | 说明 |
|------|------|
| 进程模型 | `enableFork=false`，进程内运行 |
| 默认端口 | 18789 |
| 双通道输出 | 同时发射到 `openclawConversation.responseStream` 和 `conversation.responseStream` |
| 诊断接口 | `getDiagnostics()` → `{ workspace, backend, agentName, cliPath, gatewayHost, gatewayPort, conversation_id, isConnected, hasActiveSession, sessionKey }` |

### Nanobot Agent 实现

进程内运行，CLI 阻塞模式。

**关键行为**：

| 行为 | 说明 |
|------|------|
| 进程模型 | `enableFork=false`，进程内运行 |
| 发送方式 | fire-and-forget（CLI 阻塞直到完成） |
| YOLO | 始终返回 `false`（不支持） |
| 流式输出 | 仅 `conversation.responseStream` |

### Remote Agent 实现

进程内运行，复用 OpenClaw Gateway 连接协议。

**关键行为**：

| 行为 | 说明 |
|------|------|
| 初始化 | 从数据库读取 `RemoteAgentConfig`（by `remoteAgentId`） |
| 连接状态 | 更新 `remote_agents` 表的 `status: 'connected' | 'error'` |
| Session 恢复 | 尝试 `sessionsResolve(resumeKey)`，失败回退到 `sessionsReset(conversationId)` |

## Worker 进程协议（原 TS 实现）

> **Rust 迁移说明**：以下 Worker 协议是原 TS 实现中 Node.js fork + pipe 的通信机制。Rust 重写时不沿用此模型——CLI 类 Agent 使用 `tokio::process::Command` 管理子进程（stdin/stdout 通信），Aionrs 直接作为 Rust crate 集成。此节保留为参考，帮助理解原实现的消息流转。

### 消息格式

**主进程 → Worker**：

| type | data | 说明 |
|------|------|------|
| `start` | 初始化配置 | 启动 Agent |
| `send.message` | 用户消息 | 发送消息 |
| `stop.stream` | `{}` | 停止流式输出 |

**Worker → 主进程**：

| type | data | 说明 |
|------|------|------|
| `complete` | 结果 | 操作完成 |
| `error` | 错误信息 | 操作失败 |
| `{agent}.message` | 流式事件 | Agent 特定的流式消息 |

### 请求-响应关联

- 每条请求消息携带 `pipeId`（8 字符十六进制 UUID）
- Worker 在响应中回传相同 `pipeId`
- `postMessagePromise(type, data)` 根据 `pipeId` 匹配 Promise resolve

### Worker 进程通信方式

| 运行环境 | 发送 | 接收 |
|---------|------|------|
| Node.js child_process | `process.send()` | `process.on('message')` |
| Worker Thread | `parentPort.postMessage()` | `parentPort.on('message')` |

### Worker 入口文件

| Worker 文件 | Agent 类型 | 流式事件 key | 监听的 pipe 消息 |
|------------|-----------|-------------|----------------|
| `worker/gemini.ts` | gemini | `gemini.message` | `stop.stream`, `init.history`, `send.message` |
| `worker/acp.ts` | acp | `acp.message` | `send.message` |
| `worker/aionrs.ts` | aionrs | `aionrs.message` | `stop.stream`, `init.history`, `send.message` |

## API 客户端层

> **定位**：通用 LLM API 客户端层，提供多密钥轮转 + 协议转换能力，可服务于任何需要直接调用 LLM API 的场景。当前唯一的调用方是图片生成（`imageGenCore.ts`，被 MCP 图片生成工具和 Gemini Agent 内置工具调用）。**不用于** API Key 有效性测试（Key 测试在 `modelBridge.ts` 中用原始 `fetch()` 和裸 SDK 实现，见模块 4）。

### 架构分层

| 层级 | 组件 | 能力 |
|------|------|------|
| 基类 | `RotatingApiClient<T>` | 多密钥 failover + 可重试错误自动切换 Key |
| 子类 | `OpenAIRotatingClient` | OpenAI 原生 SDK 调用 |
| 子类 | `GeminiRotatingClient` | OpenAI 格式 ↔ Gemini 原生格式（通过 `OpenAI2GeminiConverter`） |
| 子类 | `AnthropicRotatingClient` | OpenAI 格式 ↔ Anthropic 原生格式（通过 `OpenAI2AnthropicConverter`） |
| 工厂 | `ClientFactory` | 根据 provider 的 `authType` 自动创建对应客户端 |

### 多密钥轮转机制

`RotatingApiClient<T>` 抽象基类，提供多 API Key 轮转和重试能力。

**密钥管理**（`ApiKeyManager`）：

| 特性 | 说明 |
|------|------|
| 密钥格式 | 逗号或换行分隔的多个 Key |
| 初始选择 | 随机选择一个 Key |
| 黑名单 | 失败的 Key 冻结 **90 秒** |
| 环境变量同步 | 轮转时更新 `OPENAI_API_KEY` / `GEMINI_API_KEY` / `ANTHROPIC_API_KEY` |
| 状态查询 | `getStatus()` → `{ authType, envKey, current, total, keys, blacklisted }` |

**重试策略**：

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `maxRetries` | 3 | 最大重试次数 |
| `retryDelay` | 1000ms | 基础延迟（指数退避：`retryDelay * attempt`） |
| 可重试错误 | 401 / 429 / 503 / 5xx | 自动轮转 Key 后重试 |

### 客户端实现

| 客户端 | 支持的操作 | 说明 |
|--------|----------|------|
| `OpenAIRotatingClient` | `createChatCompletion`, `createImage`, `createEmbedding` | OpenAI SDK |
| `GeminiRotatingClient` | `generateContent`, `createChatCompletion` | Gemini SDK + OpenAI 协议转换 |
| `AnthropicRotatingClient` | `createChatCompletion`, `createMessage` | Anthropic SDK + OpenAI 协议转换 |

### 客户端工厂

`ClientFactory.createRotatingClient(provider, options)` 根据 `authType` 分发：

| authType | 客户端 | 代理支持 |
|----------|--------|---------|
| `USE_OPENAI` / 默认 | `OpenAIRotatingClient` | 支持 HTTP Proxy |
| `USE_GEMINI` / `USE_VERTEX_AI` | `GeminiRotatingClient` | — |
| `USE_ANTHROPIC` | `AnthropicRotatingClient` | — |

**`new-api` 平台 URL 规范化**：自动去除 `baseUrl` 中的 `/v1` 或 `/v1beta` 后缀，再按客户端类型补回正确后缀。

### 协议转换器

`ProtocolConverter<TInput, TOutput, TResponse>` 接口，在不同 LLM SDK 协议间转换。

**OpenAI → Gemini 转换**（`OpenAI2GeminiConverter`）：
- 函数名清洗：规范为 `[a-zA-Z_][a-zA-Z0-9_]*`
- 图片生成检测：识别图片生成提示后设置 `responseModalities: ['IMAGE', 'TEXT']`

**OpenAI → Anthropic 转换**（`OpenAI2AnthropicConverter`）：
- OpenAI ChatCompletion 参数 ↔ Anthropic Messages API 参数互转

## 技能系统

### AcpSkillManager（单例）

管理 Agent 的技能发现、索引和按需加载。

**技能存储路径**（优先级递减）：

| 目录 | 说明 |
|------|------|
| `_builtin/` | 内置技能 |
| `builtin-skills/` | 打包的内置技能 |
| `skills/` | 用户自定义技能 |

**技能文件格式**：每个技能为一个目录，包含 `SKILL.md` 文件。

**核心接口**：

| 方法 | 说明 |
|------|------|
| `discoverSkills(enabledSkills?)` | 扫描三个目录，发现所有技能 |
| `getSkillsIndex()` | 返回轻量列表（name + description），用于首次消息注入 |
| `getSkill(name)` | 延迟加载技能完整内容（body from SKILL.md） |
| `buildSkillsIndexText(skills)` | 格式化技能索引，含 `[LOAD_SKILL: skill-name]` 协议说明 |
| `detectSkillLoadRequest(content)` | 解析 Agent 输出中的 `[LOAD_SKILL: ...]` 请求 |

**技能注入策略**：

| Agent 类型 | 注入方式 |
|-----------|---------|
| ACP / Codex | 首条消息注入技能**索引**（name + description），Agent 按需通过 `[LOAD_SKILL]` 加载完整内容 |
| Gemini | System Instructions 注入技能**完整内容** |

### 技能数据模型

```
SkillDefinition {
  name: string            // 技能名称
  description: string     // 一行描述
  location: string        // 文件系统路径
  body: string | null     // 延迟加载的完整内容
}

SkillIndex {
  name: string            // 技能名称
  description: string     // 一行描述
}
```

## 消息中间件

### MessageMiddleware

在每条完成的 Agent 消息（`status === 'finish'`）上运行的后处理管道。

**处理步骤**：

1. **清理 think 标签**：移除 `<think>...</think>` 和 `<thinking>...</thinking>` 标签
2. **Cron 命令检测与执行**：解析并执行嵌入在 Agent 响应中的定时任务命令
3. **返回处理结果**：`{ message, displayMessage?, systemResponses }`

### Cron 命令协议

Agent 可在响应文本中嵌入结构化的 Cron 命令标签：

**创建定时任务**：
```
[CRON_CREATE]
name: 每日代码审查
schedule: 0 9 * * MON
schedule_description: 每周一上午 9 点
message: 请审查本周的代码变更
[/CRON_CREATE]
```

**列出定时任务**：
```
[CRON_LIST]
```

**删除定时任务**：
```
[CRON_DELETE: job-id]
```

**检测工具**（`CronCommandDetector`）：

| 函数 | 说明 |
|------|------|
| `detectCronCommands(text)` | 解析所有 Cron 命令 |
| `hasCronCommands(text)` | 快速检测是否包含 Cron 命令 |
| `stripCronCommands(text)` | 移除 Cron 命令标签，返回清理后的文本 |

### 系统指令构建工具

| 函数 | 用途 | 说明 |
|------|------|------|
| `buildSystemInstructions(config)` | Gemini System Prompt | 注入完整技能内容 |
| `prepareFirstMessage(content, config)` | 首条消息前缀 | `[Assistant Rules]` + 完整技能内容 |
| `prepareFirstMessageWithSkillsIndex(content, config)` | 首条消息前缀（ACP/Codex） | `[Assistant Rules]` + 技能索引 |
| `buildSystemInstructionsWithSkillsIndex(config)` | Gemini System Prompt（索引版） | 仅注入技能索引 |

## REST API

### POST /api/bedrock/test-connection

测试 AWS Bedrock 凭证连接。

**需要认证**：是

**请求体**：

```json
{
  "bedrockConfig": {
    "authMethod": "accessKey",
    "accessKeyId": "AKIA...",
    "secretAccessKey": "...",
    "region": "us-east-1"
  }
}
```

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `bedrockConfig.authMethod` | `"accessKey" \| "profile"` | 是 | 认证方式 |
| `bedrockConfig.accessKeyId` | `string` | 条件 | Access Key ID（accessKey 方式必填） |
| `bedrockConfig.secretAccessKey` | `string` | 条件 | Secret Access Key（accessKey 方式必填） |
| `bedrockConfig.profile` | `string` | 条件 | AWS Profile 名称（profile 方式必填） |
| `bedrockConfig.region` | `string` | 是 | AWS Region |

**实现说明**：使用 `BedrockContentGenerator.countTokens()` 作为轻量连接测试，不消耗配额。默认测试模型：`anthropic.claude-sonnet-4-5-20250929-v1:0`。

**成功响应** `200`：

```json
{
  "success": true,
  "msg": "Connection successful"
}
```

**错误响应**：

| 状态码 | 场景 |
|--------|------|
| 400 | 配置字段缺失 |
| 403 | 未认证 |
| 422 | Bedrock 凭证无效或无权限 |
| 500 | 服务器内部错误 |

> **设计决策**：原实现中临时修改 `process.env` 中的 AWS 环境变量，存在并发安全问题。Rust 重写时每次测试构造独立的 AWS credential provider，不污染全局环境。

---

### GET /api/gemini/subscription-status

查询 Gemini CLI 订阅状态。

**需要认证**：是

**查询参数**：

| 参数 | 类型 | 说明 |
|------|------|------|
| `proxy` | `string` | 可选，HTTP 代理 |

**成功响应** `200`：

```json
{
  "success": true,
  "data": {
    "subscriptionStatus": "active"
  }
}
```

**错误响应**：

| 状态码 | 场景 |
|--------|------|
| 403 | 未认证 |
| 502 | Gemini 服务不可达 |
| 500 | 服务器内部错误 |

---

### GET /api/remote-agents

获取所有远程 Agent 配置。

**需要认证**：是

**成功响应** `200`：

```json
{
  "success": true,
  "data": [
    {
      "id": "ra-uuid-xxx",
      "name": "远程服务器",
      "protocol": "acp",
      "url": "wss://remote.example.com",
      "authType": "bearer",
      "status": "connected",
      "lastConnectedAt": 1712345678000,
      "createdAt": 1712345600000,
      "updatedAt": 1712345678000
    }
  ]
}
```

> **设计决策**：`authToken` 在列表响应中不返回（脱敏），仅在获取单个详情时返回脱敏版本。`devicePrivateKey` 永不返回给客户端。

**错误响应**：

| 状态码 | 场景 |
|--------|------|
| 403 | 未认证 |
| 500 | 服务器内部错误 |

---

### GET /api/remote-agents/:id

获取单个远程 Agent 配置。

**需要认证**：是

**成功响应** `200`：

```json
{
  "success": true,
  "data": {
    "id": "ra-uuid-xxx",
    "name": "远程服务器",
    "protocol": "acp",
    "url": "wss://remote.example.com",
    "authType": "bearer",
    "authToken": "***abcd",
    "allowInsecure": false,
    "avatar": null,
    "description": "生产环境 Agent",
    "status": "connected",
    "lastConnectedAt": 1712345678000,
    "createdAt": 1712345600000,
    "updatedAt": 1712345678000
  }
}
```

**错误响应**：

| 状态码 | 场景 |
|--------|------|
| 403 | 未认证 |
| 404 | 远程 Agent 不存在 |
| 500 | 服务器内部错误 |

---

### POST /api/remote-agents

创建远程 Agent 配置。

**需要认证**：是

**请求体**：

```json
{
  "name": "远程服务器",
  "protocol": "openclaw",
  "url": "wss://remote.example.com",
  "authType": "bearer",
  "authToken": "token-xxx",
  "allowInsecure": false,
  "avatar": null,
  "description": "生产环境 Agent"
}
```

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `name` | `string` | 是 | 显示名称 |
| `protocol` | `RemoteAgentProtocol` | 是 | 通信协议 |
| `url` | `string` | 是 | WebSocket URL |
| `authType` | `RemoteAgentAuthType` | 是 | 认证方式 |
| `authToken` | `string` | 否 | 认证 Token |
| `allowInsecure` | `boolean` | 否 | 允许不安全连接 |
| `avatar` | `string` | 否 | 头像 URL |
| `description` | `string` | 否 | 描述 |

**副作用**：
- 当 `protocol === 'openclaw'` 时，自动生成 Ed25519 设备密钥对（`deviceId`、`devicePublicKey`、`devicePrivateKey`）

**成功响应** `201`：

```json
{
  "success": true,
  "data": { /* 完整 RemoteAgentConfig 对象 */ }
}
```

**错误响应**：

| 状态码 | 场景 |
|--------|------|
| 400 | 必填字段缺失、URL 格式无效 |
| 403 | 未认证 |
| 500 | 服务器内部错误 |

---

### PUT /api/remote-agents/:id

更新远程 Agent 配置。

**需要认证**：是

**请求体**：同创建，所有字段可选（部分更新）。

**允许更新的字段**：`name`、`protocol`、`url`、`authType`、`authToken`、`avatar`、`description`、`allowInsecure`

**成功响应** `200`：

```json
{
  "success": true,
  "data": { /* 更新后的完整对象 */ }
}
```

**错误响应**：

| 状态码 | 场景 |
|--------|------|
| 400 | 字段校验失败 |
| 403 | 未认证 |
| 404 | 远程 Agent 不存在 |
| 500 | 服务器内部错误 |

---

### DELETE /api/remote-agents/:id

删除远程 Agent 配置。

**需要认证**：是

**成功响应** `200`：

```json
{
  "success": true
}
```

**错误响应**：

| 状态码 | 场景 |
|--------|------|
| 403 | 未认证 |
| 404 | 远程 Agent 不存在 |
| 500 | 服务器内部错误 |

---

### POST /api/remote-agents/test-connection

测试远程 Agent WebSocket 连接。

**需要认证**：是

**请求体**：

```json
{
  "url": "wss://remote.example.com",
  "authType": "bearer",
  "authToken": "token-xxx",
  "allowInsecure": false
}
```

**实现说明**：
- 打开 WebSocket 连接，`open` 事件触发即视为成功
- 10 秒超时
- URL 协议校验：仅允许 `ws://` 或 `wss://`（SSRF 防护）

**成功响应** `200`：

```json
{
  "success": true
}
```

**错误响应**：

| 状态码 | 场景 |
|--------|------|
| 400 | URL 协议不合法 |
| 403 | 未认证 |
| 408 | 连接超时（10 秒） |
| 502 | WebSocket 连接失败 |
| 500 | 服务器内部错误 |

---

### POST /api/remote-agents/:id/handshake

OpenClaw 协议设备配对握手。

**需要认证**：是

**实现说明**：
- 仅对 `protocol === 'openclaw'` 有意义
- 创建 `OpenClawGatewayConnection`，监听 `onHelloOk` / `onConnectError` 事件
- 15 秒超时

**成功响应** `200`：

```json
{
  "success": true,
  "data": {
    "status": "ok"
  }
}
```

**可能的 status 值**：

| status | 说明 | 建议操作 |
|--------|------|---------|
| `ok` | 配对成功 | — |
| `pending_approval` | 等待远端审批 | 稍后重试 |
| `error` | 配对失败 | 检查配置 |

**错误响应**：

| 状态码 | 场景 |
|--------|------|
| 403 | 未认证 |
| 404 | 远程 Agent 不存在 |
| 408 | 握手超时（15 秒） |
| 500 | 服务器内部错误 |

## IPC 接口（Electron → 后端）

### Bedrock 连接测试

| 通道 | 目标协议 | 说明 |
|------|---------|------|
| `ipcBridge.bedrock.testConnection` | HTTP `POST /api/bedrock/test-connection` | 测试 Bedrock 凭证 |

### Gemini 订阅查询

| 通道 | 目标协议 | 说明 |
|------|---------|------|
| `ipcBridge.gemini.subscriptionStatus` | HTTP `GET /api/gemini/subscription-status` | 查询订阅状态 |

### Remote Agent 管理

| 通道 | 目标协议 | 说明 |
|------|---------|------|
| `ipcBridge.remoteAgent.list` | HTTP `GET /api/remote-agents` | 列出远程 Agent |
| `ipcBridge.remoteAgent.get` | HTTP `GET /api/remote-agents/:id` | 获取单个配置 |
| `ipcBridge.remoteAgent.create` | HTTP `POST /api/remote-agents` | 创建远程 Agent |
| `ipcBridge.remoteAgent.update` | HTTP `PUT /api/remote-agents/:id` | 更新远程 Agent |
| `ipcBridge.remoteAgent.delete` | HTTP `DELETE /api/remote-agents/:id` | 删除远程 Agent |
| `ipcBridge.remoteAgent.testConnection` | HTTP `POST /api/remote-agents/test-connection` | 测试连接 |
| `ipcBridge.remoteAgent.handshake` | HTTP `POST /api/remote-agents/:id/handshake` | 设备配对 |

## 数据模型

### RemoteAgentConfig

远程 Agent 配置：

```
RemoteAgentConfig {
  id: string                          // UUID
  name: string                        // 显示名称
  protocol: RemoteAgentProtocol       // 通信协议
  url: string                         // WebSocket URL
  auth_type: RemoteAgentAuthType      // 认证方式
  auth_token: string | null           // 认证 Token（加密存储）
  allow_insecure: boolean             // 允许不安全连接
  avatar: string | null               // 头像 URL
  description: string | null          // 描述
  device_id: string | null            // OpenClaw 设备 ID
  device_public_key: string | null    // Ed25519 公钥
  device_private_key: string | null   // Ed25519 私钥（加密存储）
  device_token: string | null         // 设备 Token
  status: RemoteAgentStatus           // 连接状态
  last_connected_at: number | null    // 最后连接时间
  created_at: number                  // 创建时间 (ms)
  updated_at: number                  // 更新时间 (ms)
}
```

### AcpModelInfo

ACP 后端模型信息：

```
AcpModelInfo {
  model_id: string                    // 模型标识
  model_name: string | null           // 模型显示名称
  provider: string | null             // 提供商名称
}
```

### AcpSessionConfigOption

ACP 会话配置选项：

```
AcpSessionConfigOption {
  config_id: string                   // 配置项 ID
  label: string                       // 显示标签
  value: string                       // 当前值
  options: string[] | null            // 可选值列表（null 表示自由输入）
}
```

## 枚举类型

### RemoteAgentProtocol

```
RemoteAgentProtocol = "openclaw" | "zeroclaw" | "acp"
```

### RemoteAgentAuthType

```
RemoteAgentAuthType = "bearer" | "password" | "none"
```

### RemoteAgentStatus

```
RemoteAgentStatus = "unknown" | "connected" | "pending" | "error"
```

### AgentKillReason

```
AgentKillReason = "idle_timeout"
```

## 模块依赖

- **依赖**：
  - `02-database`：远程 Agent 配置持久化（`remote_agents` 表）
  - `03-auth`：API 认证中间件
  - `04-system-settings`：获取模型提供商配置（API Key、Base URL 等）
  - `11-cron`：Cron 命令的创建/删除/列出
  - `12-mcp`：Gemini Agent 的 MCP 服务器加载
  - `13-extension`：扩展注入的 MCP 服务器和技能

- **被依赖**：
  - `05-conversation`：会话模块通过 `IAgentManager` / `IWorkerTaskManager` 接口驱动 Agent
  - `09-channel`：通道消息转发到 Agent
  - `10-team`：团队模式的 MCP 配置注入

## 候选公共类型

| 类型 | 来源 | 说明 |
|------|------|------|
| `RemoteAgentConfig` | remote agent | 远程 Agent 配置，数据库和 API 共用 |
| `RemoteAgentProtocol` | remote agent | 远程通信协议枚举 |
| `RemoteAgentAuthType` | remote agent | 认证方式枚举 |
| `RemoteAgentStatus` | remote agent | 连接状态枚举 |
| `AgentKillReason` | task manager | 任务终止原因 |
| `SkillDefinition` / `SkillIndex` | skill system | 技能定义，技能系统和 Agent 共用 |

## 常量

### Agent 生命周期

| 常量 | 值 | 说明 |
|------|-----|------|
| 默认空闲超时 | 5 分钟 | 可通过 `acp.agentIdleTimeout`（分钟）配置 |
| 空闲检查间隔 | 1 分钟 | `tokio::time::interval` |
| 空闲清理条件 | `type === 'acp'` && `status === 'finished'` | 仅 ACP 类型参与空闲清理 |
| ACP kill 优雅等待 | 500ms | `agent.kill()` → 500ms → `super.kill()` |

### 连接测试

| 常量 | 值 | 说明 |
|------|-----|------|
| Remote Agent 连接超时 | 10 秒 | WebSocket `open` 等待 |
| Remote Agent 握手超时 | 15 秒 | OpenClaw 配对等待 |
| Bedrock 测试模型 | `anthropic.claude-sonnet-4-5-20250929-v1:0` | 轻量连接测试 |

### API 客户端

| 常量 | 值 | 说明 |
|------|-----|------|
| 密钥黑名单时长 | 90 秒 | 失败 Key 的冻结期 |
| 默认最大重试 | 3 次 | `RotatingApiClient` 重试上限 |
| 默认重试延迟 | 1000ms | 指数退避基础值 |
| OpenClaw 默认端口 | 18789 | Gateway 默认端口 |

## Rust 迁移备注

### 整体策略

Rust 后端作为 **CLI 进程编排器**，不重写各 AI 后端的内部协议：

| Agent | Rust 实现方式 | 通信 |
|-------|-------------|------|
| ACP（Claude/Qwen/CodeBuddy 等） | `tokio::process::Command` 启动 CLI | stdin/stdout pipe |
| Gemini（aioncli-core） | `tokio::process::Command` 启动 CLI | stdin/stdout pipe |
| Aionrs | **直接引入 Rust crate**（自研 Rust 库） | 函数调用，无 IPC 开销 |
| OpenClaw | `tokio::process::Command` 启动 CLI | stdin/stdout pipe |
| Nanobot | `tokio::process::Command` 启动 CLI | stdin/stdout pipe |
| Remote | **Rust 重实现 WebSocket 协议层** | `tokio-tungstenite` |

### 具体建议

1. **CLI 子进程管理**：统一使用 `tokio::process::Command` 管理 ACP/Gemini/OpenClaw/Nanobot CLI，通过 stdin/stdout 通信。封装通用的 `CliAgentProcess` 结构处理进程启动、消息收发、优雅终止
2. **Aionrs 集成**：作为 Cargo workspace 的依赖 crate 直接引入，无需子进程调用。这是性能最优的集成方式
3. **Remote Agent 协议**：使用 `tokio-tungstenite` 重实现 WebSocket 连接层（复用 OpenClaw Gateway 连接协议）
4. **通用 LLM API 客户端层**：实现 `RotatingApiClient`——`ApiKeyManager` 用 `Arc<RwLock<Vec<ApiKeyEntry>>>` + `Instant` 时间戳黑名单；协议转换用 `ProtocolConverter` trait + `serde` JSON 转换。当前调用方为图片生成，但架构设计为通用层，可服务未来其他直接 LLM 调用场景
5. **技能系统**：技能发现用 `tokio::fs` 遍历目录，`SkillManager` 用 `RwLock<HashMap<String, SkillDefinition>>` 缓存
6. **Cron 命令检测**：用 `regex` crate 实现标签解析，独立为 `cron_command_detector` 模块
7. **Ed25519 密钥生成**：使用 `ed25519-dalek` crate，私钥加密存储
8. **SSRF 防护**：Remote Agent URL 校验白名单协议（`ws://`、`wss://`），可考虑进一步限制内网 IP 段
9. **并发安全**：原实现中 Bedrock 测试临时修改全局环境变量（`process.env`），Rust 中每次构造独立 `CredentialsProviderChain`，无全局状态污染
10. **空闲超时**：使用 `tokio::time::interval` + `DashMap::retain()` 扫描，仅清理 CLI 子进程类已完成任务
