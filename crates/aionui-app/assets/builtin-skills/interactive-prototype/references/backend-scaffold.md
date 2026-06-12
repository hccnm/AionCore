# Backend Scaffold

这是一套只服务 `backend` 模式的轻量后台骨架。

目标很明确：

- 让后台原型不再从空白 HTML 起步
- 提供稳定的管理端页壳、列表区块、详情区块、表单区块
- 提供最小可用的点击交互
- 不把 `prototype-studio` 的整套样式体系搬进来

## 包含文件

- `assets/backend_shell.html`
- `assets/backend_list_section.html`
- `assets/backend_detail_section.html`
- `assets/backend_form_section.html`
- `assets/backend_runtime.js`

## 什么时候用

当任务符合这些特征时，优先复用这套骨架：

- admin / 后台 / 管理端 / 审核台 / 运营台
- CRM / ERP / 订单中心 / 财务台 / 配置中心
- 信息密度高
- 重点是导航、筛选、列表、详情、批量操作、审核流

## 怎么用

1. 把 `backend_shell.html` 的 `<style>` 和壳体结构 inline 到最终 HTML
2. 用 `backend_list_section.html` / `backend_detail_section.html` / `backend_form_section.html` 组装主要内容
3. 把 `backend_runtime.js` inline 到最终 HTML 的 `</body>` 前
4. 用 `data-open` / `data-close` / `data-toast` / `data-tab-group` 接轻交互

## 支持的轻交互

- `data-nav`
- `data-open`
- `data-close`
- `data-toast`
- `data-tab-group`

它适合：

- 抽屉打开 / 关闭
- 弹窗打开 / 关闭
- tab 切换
- toast 提示
- 页面跳转

## 使用原则

- 后台模式默认无浏览器壳
- 直接表达系统本身，不做“浏览器里再套浏览器”
- 先保证导航、筛选、列表、详情、状态的清晰度
- 先改业务内容和区块组合，不要一上来重写整套壳

## 边界

这不是完整 CSS framework，也不是 `prototype-studio` 的整库移植。

它只提供：

- 后台模式的稳定起步骨架
- 最小但足够的交互 runtime
- 一套统一命名的 `ip-*` class

如果任务是 `web` 或 `app`，不要默认使用它。
