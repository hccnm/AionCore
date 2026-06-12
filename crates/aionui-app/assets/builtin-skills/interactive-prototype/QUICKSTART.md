# Interactive Prototype Quickstart

## 1. 安装为本地 skill

把整个目录放进本地 skills 目录，不要只复制 `SKILL.md`。

示例：

```bash
mkdir -p ~/.agents/skills
cp -R interactive-prototype ~/.agents/skills/
```

或：

```bash
mkdir -p ~/.codex/skills
cp -R interactive-prototype ~/.codex/skills/
```

## 2. 直接使用

示例指令：

- `$interactive-prototype 根据这份 PRD 做一个后台原型`
- `$interactive-prototype 做一个可点击的 iOS onboarding 原型`
- `$interactive-prototype 基于这个 prototype 目录执行 update`

## 3. 管理原型目录

先初始化：

```bash
python scripts/init_prototype.py \
  --target /path/to/prototype-dir \
  --name "订单中心原型"
```

然后让 agent 执行：

- `init`
- `add`
- `update`

并维护：

- `prototype.json`
- `pages/*.html`

## 4. 交付给别人

推荐两种方式：

1. 整个目录作为独立 Git 仓库交付
2. 整个目录打包成 zip 交付

必须一起交付这些内容：

- `SKILL.md`
- `assets/`
- `references/`
- `prompts/`
- `scripts/`

## 5. 最低要求

- agent 能读取本地文件
- Python 3
- 如需自动验证，额外安装 Playwright Python 依赖

```bash
pip install playwright
playwright install chromium
```

## 6. 典型输入

- 一份 PRD，生成单文件 HTML 原型
- 多份 PRD，初始化一个 prototype 目录
- 已有 prototype 目录，执行增量 `add / update`
