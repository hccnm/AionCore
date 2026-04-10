# 15 - Pet 系统

## 概述

桌面宠物（Desktop Pet）：在用户屏幕上显示一个动画角色，实时反映 AI 的工作状态（思考、回答、完成、出错等），空闲时展示随机动画和睡眠序列，响应用户点击/拖拽交互，并可选地在浮窗气泡中展示 AI 工具调用确认弹窗。

**源码位置**：`process/pet/`（`petManager.ts`、`petStateMachine.ts`、`petIdleTicker.ts`、`petEventBridge.ts`、`petConfirmManager.ts`、`petTypes.ts`）

> **设计决策 - 职责迁移**：Pet 系统是高度 UI/桌面绑定的功能。原实现中状态机运行在 Electron 主进程，通过 IPC 驱动独立的 BrowserWindow 渲染动画。Rust 重写后不存在"主进程"概念，后端是纯 HTTP/WebSocket 服务。因此 Pet 的核心逻辑（状态机、空闲计时、眼球追踪、拖拽、动画渲染）**全部迁移到前端**，后端仅负责：(1) Pet 相关设置的持久化（已在模块 04 系统设置中覆盖），(2) AI 活动事件的推送（已在模块 06/07 中覆盖）。后端不需要新增 Pet 专属的接口或逻辑。

## 子模块划分

| 子模块 | 原始源码 | 迁移策略 |
|--------|---------|---------|
| 宠物管理器 | `petManager.ts` | 不迁移至后端 — 窗口创建/销毁/拖拽/调整大小均为 Electron/前端 UI 功能 |
| 状态机 | `petStateMachine.ts` | 不迁移至后端 — 纯 UI 状态逻辑，前端根据 AI 事件流自行驱动 |
| 空闲计时器 | `petIdleTicker.ts` | 不迁移至后端 — 光标追踪、眼球运动、空闲/打哈欠/睡觉全是前端本地行为 |
| 事件桥接 | `petEventBridge.ts` | 不迁移至后端 — 前端订阅 AI 流式事件后本地映射为 Pet 状态 |
| 确认气泡管理 | `petConfirmManager.ts` | 不迁移至后端 — 工具调用确认已由对话/AI 模块通过 WebSocket 推送，前端自行决定在聊天面板还是 Pet 气泡中渲染 |
| 类型定义 | `petTypes.ts` | 不迁移至后端 — 状态/优先级/计时等纯前端常量 |

---

## 后端无需新增接口

Pet 系统所需的后端能力已被其他模块完全覆盖：

### 1. Pet 设置持久化（已在模块 04 系统设置中定义）

| 已有接口 | 说明 |
|---------|------|
| `system-settings:get-pet-enabled` / `set-pet-enabled` | 开关桌面宠物 |
| `system-settings:get-pet-size` / `set-pet-size` | 宠物尺寸（200 / 280 / 360） |
| `system-settings:get-pet-dnd` / `set-pet-dnd` | 免打扰模式 |
| `system-settings:get-pet-confirm-enabled` / `set-pet-confirm-enabled` | 确认气泡开关 |

> 这些设置项在原实现中通过 `ProcessConfig`（key-value 存储）持久化，已在 `04-system-settings.md` 中归入系统设置键值对。

### 2. AI 活动事件（已在模块 06/07 中定义）

前端通过 WebSocket 接收 AI 流式响应事件，其中包含 Pet 状态机所需的所有信号：

| AI 事件类型 | Pet 状态映射 | 说明 |
|------------|-------------|------|
| `thinking` / `thought` | `thinking` | AI 正在思考 |
| `text` / `content` | `working` | AI 正在输出内容 |
| `finish` | `done` | AI 回合完成 |
| `error` | `error` | AI 执行出错 |
| `confirmation.add` | `notification` | 工具调用需要用户确认 |
| 用户发送消息 | `thinking`（预先） | 消息发送后立即进入思考态 |

> 前端无需后端额外推送 Pet 专属事件 — 上述事件已包含在 AI 流式响应和确认推送中，前端的 PetEventBridge 直接在本地消费。

### 3. 工具调用确认（已在模块 05/06 中定义）

确认弹窗的数据流：
- 后端推送 `confirmation.add` / `confirmation.update` / `confirmation.remove` 事件
- 前端接收后，根据 `pet.confirmEnabled` 设置决定在哪里渲染（聊天面板 or Pet 浮窗气泡）
- 用户响应后，前端调用已有的确认回复接口

---

## 前端迁移指引

以下内容虽非后端 API 规范，但记录了 Pet 系统的核心机制，供前端重实现时参考。

### 状态机

共 21 种状态，按优先级分层（高优先级可抢占低优先级，同优先级受最小显示时间保护）：

| 优先级 | 状态 | 触发条件 | 备注 |
|--------|------|---------|------|
| 10 | `dragging` | 用户拖拽 | 最高优先级，拖拽期间冻结其他状态 |
| 8 | `error` | AI 执行出错 | |
| 7 | `notification` | 工具调用确认到达 | |
| 6 | `sweeping` | 随机活动动画 | |
| 5 | `done`, `happy`, `attention` | AI 完成 / 用户右键"摸摸" / 单击 | |
| 4 | `carrying`, `juggling`, `building` | 随机活动动画 / 连续点击 ≥4 次 | |
| 3 | `working` | AI 正在输出 | |
| 2 | `thinking`, `waking`, `poke-left`, `poke-right` | AI 思考 / 鼠标唤醒 / 双击 | |
| 1 | `idle`, `random-look`, `random-read` | 默认 / 空闲 20 秒随机 | |
| 0 | `yawning`, `dozing`, `sleeping` | 空闲 60 秒 / 哈欠后 / 空闲 10 分钟 | |

**状态转换规则**：
- 高优先级可立即抢占低优先级
- 同优先级切换受 `MIN_DISPLAY_MS` 保护（未到时间则排队等待）
- 多数状态有 `AUTO_RETURN` 配置，超时后自动回到 `idle`（`yawning` → `dozing`）
- 免打扰模式（DnD）下仅允许 `dragging`

### 空闲行为序列

```
idle → (20s 鼠标不动) → random-look 或 random-read
     → (60s 鼠标不动) → yawning
     → (哈欠结束) → dozing
     → (10min 鼠标不动) → sleeping
     → (鼠标移动) → waking → idle
```

### 眼球追踪

空闲状态下，Pet 眼睛跟随鼠标方向移动（50ms 采样），计算相对于 Pet 中心的方向向量，映射为 SVG viewBox 坐标偏移（左右 ±3 单位，上 1.3、下 1 单位），身体随之微幅偏移和旋转。

### 点击交互

| 点击次数 | 反应 |
|---------|------|
| 1 次 | `attention`（小惊讶） |
| 2-3 次 | `poke-left` / `poke-right`（按点击侧方向摇晃） |
| ≥ 4 次 | `juggling`（手忙脚乱） |

### 确认气泡窗口

当 `pet.confirmEnabled` 为 `true` 时，AI 工具调用确认不在聊天面板中显示，而是弹出一个贴近 Pet 的浮窗。浮窗支持拖拽重定位（记忆当前会话位置），无确认请求时自动销毁。

---

## 数据模型

### PetState（前端枚举）

```
idle | thinking | working | done | happy | error | dragging | attention |
poke-left | poke-right | notification | random-look | random-read |
yawning | dozing | sleeping | waking | sweeping | juggling | building | carrying
```

### PetSize（前端枚举）

```
200 | 280 | 360
```

### EyeMoveData（前端内部）

```
{
  eyeDx: number      // 眼球水平偏移（SVG 单位）
  eyeDy: number      // 眼球垂直偏移（SVG 单位）
  bodyDx: number     // 身体水平偏移 (= eyeDx × 0.35)
  bodyRotate: number  // 身体旋转角度 (= eyeDx × 0.6)
}
```

---

## 模块依赖

| 依赖方向 | 模块 | 说明 |
|---------|------|------|
| 使用 | 04 - 系统设置 | 读写 Pet 相关设置项 |
| 监听 | 06 - AI 后端集成 | 订阅 AI 流式事件映射 Pet 状态 |
| 监听 | 07 - 实时通信 | 通过 WebSocket 接收 AI 事件推送 |
| 监听 | 05 - 会话与消息管理 | 接收工具调用确认事件 |

> Pet 系统是纯消费者，不被其他模块依赖。

---

## 候选公共类型

无 — Pet 系统的所有类型（PetState、PetSize、EyeMoveData 等）仅在前端使用，不涉及后端公共 crate。

---

## 总结

Pet 系统是 AionUi 中最具桌面特色的功能，但从后端架构角度看，它是一个纯前端模块。Rust 后端无需为 Pet 新增任何接口、服务或数据模型。所需的后端能力（设置持久化、AI 事件推送、确认路由）已在其他模块中完整定义。前端重实现时，将原 Electron 主进程中的状态机、空闲计时器、事件桥接逻辑移至浏览器端 JavaScript 即可。
