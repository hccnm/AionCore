# Prototype Projects

这一页定义 `Interactive Prototype` 的轻量项目管理机制。

## 目标目录结构

```text
prototype/
  prototype.json
  pages/
    index.html
    ...
```

默认只要求这两层：

- `prototype.json`
- `pages/*.html`

原型页面默认应该保持**自包含 HTML**。不要为了管理模式强行复制一整套外部样式基座。

## Action Modes

### init

- 初始化目标原型目录
- 创建 `prototype.json`
- 创建 `pages/index.html`
- 追加一条 `notes`，`action: "init"`

### add

- 读取新增 PRD
- 更新页面清单
- 只新增或修改受影响页面
- 追加一条 `notes`，`action: "add"`

### update

- 读取更新后的 PRD
- 页面 id 和文件名尽量稳定
- 只改受影响页面
- 追加一条 `notes`，`action: "update"`

## prototype.json 格式

```json
{
  "name": "订单中心原型",
  "sourcePrds": [
    "/你的业务仓库/docs/prd/order-center/overview.md"
  ],
  "pages": [
    {
      "id": "index",
      "title": "首页",
      "file": "pages/index.html"
    }
  ],
  "notes": [
    {
      "time": "2026-04-29 17:00:00 +08:00",
      "action": "init",
      "summary": "初始化原型目录",
      "details": [
        "创建 prototype.json 和 pages/index.html。",
        "当前版本按自包含 HTML 原型交付。"
      ],
      "pages": ["index"],
      "sourcePrds": []
    }
  ]
}
```

## 字段规则

- `time`：`YYYY-MM-DD HH:mm:ss +08:00`
- `action`：`init` / `add` / `update`
- `summary`：一句话摘要
- `details`：2 到 5 条具体说明
- `pages`：本次新增或修改的页面 id
- `sourcePrds`：本次直接使用的 PRD 路径

## 约束

- 不要把原始 PRD 文本复制进原型目录
- 不要因为局部变更重写整个原型
- 不要大规模改名已有页面
- `notes` 必须按时间顺序追加
