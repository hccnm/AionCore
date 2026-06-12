# Prototype Rules

把需求压缩成原型时，按这份规则做。

## 1. 先选模式

### 后台

适用：

- admin / 后台 / 管理端 / 审核台 / 订单中心 / CRM / ERP
- 信息密度高
- 以表格、表单、详情、筛选、批量操作为主

默认：

- 无浏览器壳
- 直接表达系统本身
- 优先复用 `backend_shell.html` + `backend_runtime.js`
- 强调导航、内容区、筛选区、状态区、详情区

### App

适用：

- iOS / Android / mobile app
- onboarding / task / session / profile / settings

默认：

- iOS 优先 `IosFrame`
- Android 用 `AndroidFrame`

### Web

适用：

- dashboard / SaaS / browser / workspace / 产品 Web 体验
- 多栏布局，但不是典型后台管理页

默认：

- 默认无浏览器壳
- 需要强调“浏览器中的产品感”时才用 `BrowserWindow`

## 2. 再选交付形态

### Overview

适用：

- 看全貌
- 比较多个页面
- 设计 review

骨架：

```jsx
<DesignCanvas title="Flow Overview" columns={3}>
  <Variation label="Home">...</Variation>
  <Variation label="Detail">...</Variation>
  <Variation label="Settings">...</Variation>
</DesignCanvas>
```

### Flow Demo

适用：

- 走一遍用户路径
- 点击切换状态
- 演示主功能

骨架：

```jsx
function PrototypeApp() {
  const [screen, setScreen] = React.useState('home');
  const [modal, setModal] = React.useState(null);
  const [tab, setTab] = React.useState('overview');
}
```

## 3. PRD -> 原型压缩法

从 PRD 里抽这 6 项：

1. user
2. goal
3. primary flow
4. screens
5. key actions
6. must-have content

默认只做：

- 1 条主流程
- 4 到 6 个关键屏
- 1 套清晰视觉系统

不要一次做成完整生产应用。

## 4. App 原型专属规则

- 默认单文件 inline React
- 不要外链本地 JSX 文件
- 主按钮、tab、返回、卡片都要可点
- 可点击区域最小 `44x44`
- 顶部第一屏不要被 status bar / island 压住

## 5. 后台原型专属规则

- 默认无壳
- 优先从轻量后台 scaffold 起步，而不是从空白 HTML 起步
- 主导航必须清晰
- 需要有筛选、列表、详情或操作区中的至少两个
- 适合表达复杂状态、审核流、批量操作、运营工作台
- 如果只是局部业务差异，改区块内容，不要重写整套后台壳
- 不要为了“页面完整”牺牲信息密度

## 6. Web 原型专属规则

- 明确主导航
- 明确内容主区域
- 明确状态或辅助面板
- 原型重点在流程，不在营销文案
- 默认无壳
- `BrowserWindow` 仅在需要强调浏览器环境时启用

## 7. 动效规则

只保留轻动效：

- 页面切换
- drawer / modal 进入退出
- hover / press
- tab indicator

统一使用：

- `opacity`
- `transform`
- `transition`

不要引入重动画系统。

## 8. 原型完成检查

- 有没有一条完整可点的主路径
- 有没有假装“高保真”但其实不能点
- 有没有控制台错误
- 有没有写 assumptions
- 有没有明显 AI slop
