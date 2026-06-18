# 前端对接指南

> 本文档面向前端开发者，说明 SaaS 远程部署模式（Phase1）下的接口对接方式。

## 1. 背景

后端新增了三态部署模式（`local` / `desktop` / `saas`），前端远程联调使用 **SaaS 模式**，主要变化：

- 认证方式从 Cookie + CSRF 改为 **Bearer Token**
- CORS 由后端白名单控制，前端无需额外处理
- 本地/desktop 模式提供 Swagger UI 和 OpenAPI 文档端点；SaaS 模式默认不暴露文档端点
- 所有业务 `/api/*` 接口路径不变；认证初始化新增明确端点

## 2. 后端启动

前端联调时，后端需以 SaaS 模式启动：

```bash
cd AionCore

DEPLOYMENT_MODE=saas \
LISTEN_ADDR=0.0.0.0 \
LISTEN_PORT=25808 \
cargo run --bin aioncore -- --data-dir /tmp/aionui-saas
```

| 环境变量 | 说明 | 示例 |
|---|---|---|
| `DEPLOYMENT_MODE` | 部署模式 | `saas` |
| `ALLOWED_ORIGINS` | CORS 白名单。SaaS 模式必须显式配置；为空时 fail closed，不允许任意 origin | `http://localhost:5173` |
| `LISTEN_ADDR` | 监听地址，`0.0.0.0` 允许局域网访问 | `0.0.0.0` |
| `LISTEN_PORT` | 监听端口 | `25808` |

> 首次启动需初始化管理员密码，见下方「首次部署」。

## 3. 认证流程

### 3.1 登录

```
POST /login
Content-Type: application/json

{
  "username": "admin",
  "password": "admin12345678"
}
```

成功响应（200）：

```json
{
  "success": true,
  "message": "Login successful",
  "user": {
    "id": "system_default_user",
    "username": "admin"
  },
  "token": "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9..."
}
```

失败响应（401）：

```json
{
  "success": false,
  "error": "Invalid username or password",
  "code": "UNAUTHORIZED"
}
```

### 3.2 使用 Token

SaaS 模式下，登录后的业务 API 需要在请求头中携带 Token：

```
Authorization: Bearer <token>
```

公开端点例外：

- `POST /login`
- `GET /api/auth/status`
- `POST /api/auth/setup-password`（仅首次初始化）
- `POST /api/auth/refresh`（token 在 body 中）

### 3.3 刷新 Token

```
POST /api/auth/refresh
Content-Type: application/json

{
  "token": "<当前token>"
}
```

响应：

```json
{
  "success": true,
  "token": "<新token>"
}
```

### 3.4 登出

```
POST /logout
Authorization: Bearer <token>
```

### 3.5 获取当前用户

```
GET /api/auth/user
Authorization: Bearer <token>
```

### 3.6 检查系统状态（无需认证）

```
GET /api/auth/status
```

响应：

```json
{
  "success": true,
  "needs_setup": false,
  "user_count": 1,
  "is_authenticated": true
}
```

> `needs_setup: true` 表示管理员密码未设置，需先初始化。

### 3.7 前端底座要求

SaaS 远程部署不能只改单个 Settings 页面。前端需要统一封装浏览器侧的 HTTP 和 WebSocket 客户端：

- HTTP：登录后保存 JSON body 返回的 `token`，后续业务请求统一追加 `Authorization: Bearer <token>`
- 初始化：`needs_setup: true` 时走设置密码页面，不进入普通登录提交
- WebSocket：不能使用裸 `new WebSocket('/ws')`；必须先用 Bearer 调 `/api/ws-token`，再把返回值作为 `Sec-WebSocket-Protocol`
- 桌面/local 兼容：桌面端仍可保留 cookie/session 路径，但 SaaS 前端不要依赖 cookie 鉴权

## 4. 首次部署初始化

SaaS 模式首次启动时，管理员密码为空，`/api/auth/status` 返回 `needs_setup: true`。

**前端处理流程**：

1. 启动后调用 `GET /api/auth/status`
2. 如果 `needs_setup: true`，弹出设置密码页面
3. 调用 `POST /api/auth/setup-password` 设置密码（首次无需认证，仅在密码未设置时可用）
4. 密码设置后 `needs_setup` 变为 `false`，进入正常登录流程

> `/api/auth/change-password` 是已登录用户修改密码接口，需要 Bearer Token；不要用于首次部署初始化。旧的 `/api/webui/change-password` 保留给桌面/local 兼容路径。

## 5. CORS

SaaS 模式下，后端根据 `ALLOWED_ORIGINS` 环境变量控制跨域访问：

- 前端 dev server 的 origin 必须在白名单中
- `ALLOWED_ORIGINS` 为空时不放行任何跨域 origin，避免远程部署默认开放
- `Authorization`、`Content-Type`、`X-Csrf-Token` 头已预配置放行
- 带凭证的请求（credentials: true）已支持

**前端无需做任何 CORS 配置**，只要 origin 在白名单中，浏览器会自动处理。

## 6. API 文档

| 端点 | 说明 |
|---|---|
| `GET /docs` | Swagger UI 交互式文档；仅本地/desktop 模式挂载 |
| `GET /api-docs/openapi.json` | OpenAPI spec JSON；仅本地/desktop 模式挂载 |

### 导入到 API 工具

- **Apifox**：在本地/desktop 模式新建项目 → 导入 → OpenAPI → 输入 `http://localhost:25808/api-docs/openapi.json`
- **Postman**：Import → Link → 输入 URL
- **VS Code REST Client**：安装 OpenAPI 扩展后直接预览

## 7. 接口速查

> 本地/desktop 模式可访问 `/docs` 查看完整接口文档；SaaS 模式默认不暴露 docs。以下是前端常用接口。

### 认证

| 方法 | 路径 | 认证 | 说明 |
|---|---|---|---|
| POST | `/login` | 否 | 登录 |
| POST | `/logout` | 是 | 登出 |
| GET | `/api/auth/status` | 否 | 系统状态 |
| POST | `/api/auth/setup-password` | 否 | 首次初始化管理员密码 |
| GET | `/api/auth/user` | 是 | 当前用户 |
| POST | `/api/auth/change-password` | 是 | 已登录用户修改密码 |
| POST | `/api/auth/refresh` | 否 | 刷新 Token |
| GET | `/api/ws-token` | 是 | WebSocket 专用 Token |

### 对话

| 方法 | 路径 | 认证 | 说明 |
|---|---|---|---|
| POST | `/api/conversations` | 是 | 创建对话 |
| GET | `/api/conversations` | 是 | 列出对话 |
| GET | `/api/conversations/{id}` | 是 | 获取对话 |
| PATCH | `/api/conversations/{id}` | 是 | 更新对话 |
| DELETE | `/api/conversations/{id}` | 是 | 删除对话 |
| GET | `/api/conversations/{id}/messages` | 是 | 列出消息 |
| POST | `/api/conversations/{id}/messages` | 是 | 发送消息 |
| POST | `/api/conversations/{id}/cancel` | 是 | 取消操作 |
| GET | `/api/conversations/active-count` | 是 | 活跃对话数 |

### 系统

| 方法 | 路径 | 认证 | 说明 |
|---|---|---|---|
| GET | `/api/settings` | 是 | 获取设置 |
| PATCH | `/api/settings` | 是 | 更新设置 |
| GET | `/api/system/info` | 是 | 系统信息 |
| GET | `/api/providers` | 是 | 列出 Providers |
| POST | `/api/providers` | 是 | 创建 Provider |
| PUT | `/api/providers/{id}` | 是 | 更新 Provider |
| DELETE | `/api/providers/{id}` | 是 | 删除 Provider |

### 健康检查

| 方法 | 路径 | 认证 | 说明 |
|---|---|---|---|
| GET | `/health` | 否 | 健康检查 |
| GET | `/healthz` | 否 | 健康检查（alias） |

## 8. WebSocket

### 连接

```js
// 先获取 ws-token
const { ws_token } = await fetch(`${apiBaseUrl}/api/ws-token`, {
  headers: { 'Authorization': `Bearer ${token}` }
}).then(r => r.json())

// 建立 WebSocket 连接
const ws = new WebSocket(`${wsBaseUrl}/ws`, [ws_token])
```

`ws_token` 是原始 sub-protocol 值，不要加 `Bearer ` 前缀，也不要放在 query string。

### 消息格式

```json
// 发送
{ "name": "subscribe-show-open", "data": {} }

// 接收
{ "name": "realtime.error", "data": { "code": "...", "message": "..." } }
```

### 心跳

- 服务端每 30 秒发送 `ping`
- 客户端需回复 `pong`
- 60 秒无响应自动断开

## 9. 错误响应格式

所有接口的错误响应统一格式：

```json
{
  "success": false,
  "error": "错误描述",
  "code": "ERROR_CODE",
  "details": null
}
```

常见错误码：

| HTTP 状态码 | code | 说明 |
|---|---|---|
| 400 | `BAD_REQUEST` | 请求参数错误 |
| 401 | `UNAUTHORIZED` | 未认证或 Token 无效/过期 |
| 403 | `FORBIDDEN` | 无权限 |
| 404 | `NOT_FOUND` | 路由不存在 |
| 405 | `METHOD_NOT_ALLOWED` | HTTP 方法不支持 |
| 429 | `TOO_MANY_REQUESTS` | 请求过于频繁 |

## 10. 前端代码示例

### Axios 拦截器

```ts
import axios from 'axios'

const api = axios.create({ baseURL: 'http://localhost:25808' })

// 请求拦截：自动加 Token
api.interceptors.request.use((config) => {
  const token = localStorage.getItem('token')
  if (token) {
    config.headers.Authorization = `Bearer ${token}`
  }
  return config
})

// 响应拦截：401 自动跳登录
api.interceptors.response.use(
  (res) => res,
  (err) => {
    if (err.response?.status === 401) {
      localStorage.removeItem('token')
      window.location.href = '/login'
    }
    return Promise.reject(err)
  }
)
```

### Fetch 封装

```ts
async function request<T>(path: string, options: RequestInit = {}): Promise<T> {
  const token = localStorage.getItem('token')
  const res = await fetch(`http://localhost:25808${path}`, {
    ...options,
    headers: {
      'Content-Type': 'application/json',
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
      ...options.headers,
    },
  })

  if (res.status === 401) {
    localStorage.removeItem('token')
    window.location.href = '/login'
    throw new Error('Unauthorized')
  }

  return res.json()
}
```

## 11. 部署模式对比

| 特性 | Local | Desktop | SaaS |
|---|---|---|---|
| 认证 | 关闭 | Cookie + CSRF | Bearer Token |
| CORS | 允许所有 | 无（同源） | 白名单 |
| CSRF | 跳过 | 必须 | 跳过 |
| 默认 host | 127.0.0.1 | 127.0.0.1 | 0.0.0.0 |
| 适用场景 | 本地开发 | 桌面端嵌入 | 远程部署/前端联调 |
