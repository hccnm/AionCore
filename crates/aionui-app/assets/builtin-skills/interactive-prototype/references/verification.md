# Verification

交互式原型的验证至少要覆盖渲染和点击。

## 基础验证

```bash
python scripts/verify.py path/to/prototype.html
```

这个脚本会：

- 用 Playwright 打开 HTML
- 抓控制台 warning / error
- 生成截图
- 返回验证结果

## 点击流验证

对关键路径按选择器顺序点击：

```bash
python scripts/verify.py path/to/prototype.html \
  --click "[data-testid='open-detail']" \
  --click "[data-testid='confirm']" \
  --click "[data-testid='back-home']"
```

建议所有关键交互都带 `data-testid`。

## 推荐最小检查项

### App flow demo

- 首页进入二级页
- 主 CTA 完成一次动作
- tab 切换

### Web flow demo

- 打开详情抽屉 / modal
- 关键保存操作
- 左侧导航切换

## 多视口

```bash
python scripts/verify.py prototype.html --viewports 1440x900,390x844
```

## 常见失败原因

- JSX 语法错误
- 远程字体没加载完成
- 设备壳内容被 status bar 压住
- 点击元素不可见或被遮挡
- 写了静态页但没有状态切换

## 交付前结论

只有在这些都成立时，才能说“可交付”：

- HTML 能打开
- Console 干净或只有可接受 warning
- 至少一条主流程已经点通
