# Design Context

好的原型不是凭空长出来的，先从已有上下文里长。

## 上下文优先级

1. 用户的 design system / UI kit
2. 用户现有 codebase
3. 用户已发布产品的截图或 URL
4. 品牌资产
5. 竞品参考
6. fallback 风格

## 如果用户给了 codebase

优先读这些文件：

- `theme.*`
- `tokens.*`
- `colors.*`
- `global.css`
- `Button.*`
- `Card.*`
- 最接近目标页面的现有模块

要抄的是**真实值**：

- color token
- radius
- spacing scale
- font stack
- shadow pattern

## 如果用户只给了 PRD

不要卡住。按下面顺序做 fallback：

1. 先决定产品气质
2. 选一个明确视觉方向
3. 选一组可信的字体搭配
4. 用简单稳定的 spacing / color system

## fallback 时的表达方式

应该明确说：

- 这是基于通用产品语汇做的原型
- 当前重点是 flow、布局和交互
- 品牌化细节可以在下一轮继续贴近

不应该假装自己已经拿到了品牌设计系统。

## 最小提炼模板

```markdown
设计上下文提炼：

- Primary: #...
- Background: #...
- Ink: #...
- Display font: ...
- Body font: ...
- Radius: ...
- Spacing: ...
- UI vocabulary: ...
```
