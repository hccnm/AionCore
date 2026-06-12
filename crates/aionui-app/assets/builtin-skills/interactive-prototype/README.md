# Interactive Prototype

> 从 `huashu-design` 抽离出来的 prototype-only 精编版。

`Interactive Prototype` 只做一件事：

`自然语言 / PRD -> 高保真、可点击、自包含 HTML 原型`

它保留了原仓库最有价值的原型能力：

- 单指令使用方式
- agent-agnostic 技能结构
- 后台 / Web / App 原型三形态
- 自包含单文件 HTML
- 设备边框 / 可选浏览器边框
- 可点击 flow demo
- Playwright 验证
- 反 AI slop 约束

它明确移除了这些能力：

- 幻灯片 / PPTX
- MP4 / GIF / 音频导出
- 时间轴动画系统
- 信息图
- 专家评审
- 设计方向顾问和大而全 showcase 体系

## 你可以把它理解成什么

这是一个**可交付给别人用的 skill 包**，目标很单一：

- 输入：自然语言或 PRD
- 输出：高保真、可点击、自包含 HTML 原型

它不是：

- 生产级前端框架
- PPT / 视频工具
- 大而全设计平台

## 适用场景

- 给一个 PRD，生成后台管理端原型
- 给一个 PRD，生成交互式 App 原型
- 给一句产品需求，生成 Web 原型
- 做 onboarding / checkout / dashboard / settings / task flow 等可点击 demo
- 在本地双击打开 HTML 做走查或评审

## 输出契约

- 默认交付物是单文件 `.html`
- HTML 必须自包含，可直接本地打开
- 原型必须可点击，不是纯静态海报
- 支持 `后台 / web / app` 三模式
- 默认只使用轻量交互动效
- 交付前跑 Playwright 检查渲染、控制台和关键点击流

## 三种模式

### 后台

- 面向 admin / 管理端 / 运营台 / 审核台 / CRM / ERP
- 默认**无浏览器壳**
- 强调导航、筛选、表格、详情、批量操作和状态流
- 已内置一套轻量后台 scaffold，可直接复用

### Web

- 面向 SaaS 内页、workspace、产品 Web 体验、桌面工作流
- 默认**无浏览器壳**
- 只有在需要强调“浏览器中的产品感”时才加浏览器壳

### App

- 面向 iOS / Android / mobile app
- 默认带设备壳
- 强调 tab、详情、任务流、onboarding、状态切换

## 项目管理模式

这一版额外补了 4 个轻量机制：

- `init`
- `add`
- `update`
- `prototype.json`

它们只负责**管理一个原型目录的生命周期**，不会把别的页面基座强塞进原型生成流程。

也就是说：

- 原型页面依然应该是自包含 HTML
- 不默认复制一整套样式系统到目标目录
- `prototype.json` 只负责记录页面清单、来源 PRD 和变更日志

## 包结构

```text
interactive-prototype/
├── SKILL.md
├── README.md
├── LICENSE
├── agents/
│   └── openai.yaml
├── prompts/
│   ├── init.md
│   ├── add.md
│   └── update.md
├── assets/
│   ├── backend_shell.html
│   ├── backend_list_section.html
│   ├── backend_detail_section.html
│   ├── backend_form_section.html
│   ├── backend_runtime.js
│   ├── ios_frame.jsx
│   ├── android_frame.jsx
│   ├── browser_window.jsx
│   └── design_canvas.jsx
├── references/
│   ├── backend-scaffold.md
│   ├── workflow.md
│   ├── design-context.md
│   ├── prototype-rules.md
│   ├── prototype-projects.md
│   ├── react-setup.md
│   └── verification.md
├── scripts/
│   ├── init_prototype.py
│   └── verify.py
├── demos/
│   ├── backend-ops-demo.html
│   ├── ios-flow-demo.html
│   └── web-dashboard-demo.html
└── test-prompts.json
```

## 后台模式现在多了什么

这次把 `prototype-studio` 里最值得保留的后台能力，以更轻的形式并进来了：

- `assets/backend_shell.html`
- `assets/backend_list_section.html`
- `assets/backend_detail_section.html`
- `assets/backend_form_section.html`
- `assets/backend_runtime.js`

这套东西只服务 `backend` 模式，目标是让后台原型快速起壳：

- 先有稳定的页壳
- 再拼列表 / 详情 / 表单区块
- 保留抽屉 / 弹窗 / toast / tab 这类后台高频交互

但它不是把 `prototype-studio` 的整套 antd 风格基座搬进来。

也就是说：

- 后台模式不再完全从零生成
- `web / app` 模式不受影响
- 原型仍然优先交付自包含 HTML

参考：

- `references/backend-scaffold.md`
- `demos/backend-ops-demo.html`

## 交付给别人怎么用

最推荐的方式是：**把整个目录交给对方，作为一个本地 skill 安装或注册。**

不要只给 `SKILL.md`，因为这个包还依赖：

- `assets/`
- `references/`
- `scripts/`
- `prompts/`

### 方式 1：作为本地 skill 安装

适合支持 skill 机制的 agent。

对方拿到整个 `interactive-prototype/` 目录后，把它放进自己的 skills 目录，或作为一个独立仓库 clone 下来注册。

### 方式 2：直接把整个目录喂给 agent

如果对方的 agent 没有正式 skill 机制，也可以：

- 把 `SKILL.md` 当长提示词
- 同时确保 agent 能读取同目录下的 `assets/`、`references/`、`scripts/`、`prompts/`

这样也能用，只是没有正式安装稳定。

## 使用方式

### 方式 1：直接生成单个原型

示例：

- `根据这个 PRD 做一个订单中心后台原型，先覆盖首页、列表、详情和审核 flow`
- `做一个 AI 番茄钟 iOS 原型，4 个核心屏，真实可点击`
- `根据这个 PRD 做一个 CRM Web 原型，先给 overview，再给主 flow`
- `做一个设置页重构原型，保留现有视觉语汇`

### 方式 2：管理一个持续迭代的原型目录

先初始化目标目录：

```bash
python scripts/init_prototype.py \
  --target /你的业务仓库/prototype/order-center \
  --name "订单中心原型"
```

初始化后会生成：

```text
/你的业务仓库/prototype/order-center/
  prototype.json
  pages/
    index.html
```

然后让 agent 按动作执行：

#### init

```text
这是一个 init 任务。

读取这些 PRD：
- /你的业务仓库/docs/prd/order-center/overview.md
- /你的业务仓库/docs/prd/order-center/list.md

输出到：
- /你的业务仓库/prototype/order-center

基于 interactive-prototype 生成第一版原型。
请更新 prototype.json，并生成或更新 pages 下的 HTML。
```

#### add

```text
这是一个 add 任务。

基于已有原型：
- /你的业务仓库/prototype/order-center

读取这些新增 PRD：
- /你的业务仓库/docs/prd/order-center/refund.md

新增受影响的页面或 flow。
请更新 prototype.json，并只修改受影响页面。
```

#### update

```text
这是一个 update 任务。

基于已有原型：
- /你的业务仓库/prototype/order-center

这些 PRD 已更新：
- /你的业务仓库/docs/prd/order-center/list.md
- /你的业务仓库/docs/prd/order-center/detail.md

只更新受影响页面，并同步更新 prototype.json。
```

## 最小安装要求

- 能读取本地文件的 agent
- Python 3
- 如果要跑 `verify.py`，需要额外安装 Playwright Python 依赖

可选验证环境：

```bash
pip install playwright
playwright install chromium
```

## 验证

基础验证：

```bash
python scripts/verify.py demos/ios-flow-demo.html
```

带点击流验证：

```bash
python scripts/verify.py demos/ios-flow-demo.html \
  --click "[data-testid='open-session']" \
  --click "[data-testid='complete-session']" \
  --click "[data-testid='open-insights']"
```

## 备注

- 这是一个独立精编版，不会改脏原始 `huashu-design`
- 许可证沿用原仓库
- 项目管理模式只引入 `init / add / update / prototype.json`
- `backend` 模式额外补了一层轻量后台 scaffold，但没有引入整套重页面基座
- `agents/openai.yaml` 只是一个交付入口描述，不影响 skill 主体逻辑
