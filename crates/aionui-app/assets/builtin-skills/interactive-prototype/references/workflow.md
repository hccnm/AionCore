# Workflow

这是 `Interactive Prototype` 的最短执行路径。

## 原则

- 默认直接执行，不为形式感反复提问
- 输入足够时，不要停在讨论阶段
- 只有真实阻塞时才问用户
- 用户说“直接做”时，写 assumptions 继续推进

## 5 步流程

### 1. 提炼任务

先写清：

- 这是 App 还是 Web 原型
- 是 overview 还是 flow demo
- 主目标是什么
- 关键用户路径是什么
- 必须出现哪些模块

### 2. 读取上下文

优先读取：

- 现有代码 / design token
- 用户截图 / 线上产品
- 品牌资产

没有上下文时，坦白说明是 fallback，不要假装“这是品牌既有风格”。

### 3. 写 assumptions 注释

HTML 顶部必须先写：

```html
<!--
Assumptions
- 目标用户是谁
- 当前原型聚焦哪条流程
- 没有拿到哪些信息，因此怎么占位
- 为什么选 App / Web、overview / flow
-->
```

### 4. 生成原型

#### overview

- 多屏并排
- 便于看全貌
- 适合评审

#### flow demo

- 单壳体承载状态切换
- 至少一条关键路径可点击
- 返回、tab、主 CTA 都要真实切换

### 5. 验证

最少做这些：

- 打开页面
- 抓控制台错误
- 点一遍关键 flow

## 管理模式

如果任务明确是 `init / add / update`，走下面这条路径：

### init

- 如果目标目录不存在或没有 `prototype.json`，先运行 `scripts/init_prototype.py`
- 初始化 `prototype.json`
- 生成第一个 `pages/index.html`
- 把本次 PRD 写进 `sourcePrds`
- 在 `notes` 里追加一条 `init`

### add

- 读取新增 PRD
- 判断受影响页面
- 只新增或修改这些页面
- 追加一条 `add`

### update

- 读取变更后的 PRD
- 只修改受影响页面
- 保持页面 id 和文件名稳定
- 追加一条 `update`

所有管理模式任务都必须同步更新 `prototype.json`。

## 问题策略

只有在下面情况才问：

- 无法判断 App 还是 Web
- 缺少会直接改变信息架构的关键输入
- 需求里有互相冲突的硬约束

否则，直接做并在 assumptions 里写明。
