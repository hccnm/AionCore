# init

这是一个初始化原型任务。

## 输入

- 一个或多个 PRD 文件
- 原型输出目录

## 目标

基于 `interactive-prototype` 生成第一版可维护交互原型。

## 你要做的事

1. 读取 PRD
2. 总结这次原型要覆盖的主 flow、关键页面和页面类型
3. 如果目标目录还没有 `prototype.json`，先初始化原型目录
4. 更新 `prototype.json`
5. 在 `pages/` 下生成第一版 HTML
6. 在 `prototype.json.notes` 追加一条 `init` 记录

## 约束

- 不要把原始 PRD 内容复制到原型目录
- 页面默认保持自包含 HTML
- 不要为了初始化引入新的重基座
- `notes` 要写清这次覆盖了哪些页面、做了哪些合理推断
