#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
from datetime import datetime, timedelta, timezone
from pathlib import Path


TZ_SHANGHAI = timezone(timedelta(hours=8))


def now_shanghai() -> str:
    return datetime.now(TZ_SHANGHAI).strftime("%Y-%m-%d %H:%M:%S +08:00")


def write_text(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def build_prototype_json(name: str) -> dict:
    return {
        "name": name,
        "sourcePrds": [],
        "pages": [
            {
                "id": "index",
                "title": "首页",
                "file": "pages/index.html",
            }
        ],
        "notes": [
            {
                "time": now_shanghai(),
                "action": "init",
                "summary": "初始化原型目录",
                "details": [
                    "创建 prototype.json 和 pages/index.html。",
                    "当前目录用于持续维护自包含 HTML 原型。",
                ],
                "pages": ["index"],
                "sourcePrds": [],
            }
        ],
    }


def build_starter_page(name: str) -> str:
    return f"""<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>首页 · {name}</title>
  <style>
    * {{ box-sizing: border-box; }}
    body {{
      margin: 0;
      min-height: 100vh;
      padding: 40px;
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      background: #f5f1ea;
      color: #171411;
    }}
    .card {{
      max-width: 720px;
      padding: 28px;
      border-radius: 24px;
      background: rgba(255,255,255,0.82);
      box-shadow: 0 16px 50px rgba(0,0,0,0.08);
    }}
    .eyebrow {{
      font-size: 12px;
      letter-spacing: 0.12em;
      text-transform: uppercase;
      color: #7a7267;
    }}
    h1 {{
      margin: 10px 0 12px;
      font-size: 32px;
      line-height: 1.1;
    }}
    p, li {{ line-height: 1.6; color: #5a544b; }}
  </style>
</head>
<body>
  <!--
  Assumptions
  - 这是原型目录初始化后的起始页
  - 后续页面将根据 PRD 继续增量生成或更新
  - 最终交付页面默认应保持自包含 HTML
  -->
  <section class="card">
    <div class="eyebrow">Interactive Prototype</div>
    <h1>{name}</h1>
    <p>原型目录已初始化。后续请基于 PRD 执行 init / add / update。</p>
    <ol>
      <li>更新 <code>prototype.json</code></li>
      <li>新增或修改 <code>pages/*.html</code></li>
      <li>保持页面自包含、可点击、可验证</li>
    </ol>
  </section>
</body>
</html>
"""


def main() -> None:
    parser = argparse.ArgumentParser(description="Initialize an interactive-prototype project directory.")
    parser.add_argument("--target", required=True, help="Target prototype directory")
    parser.add_argument("--name", default="新原型", help="Prototype name")
    args = parser.parse_args()

    target = Path(args.target).expanduser().resolve()
    pages_dir = target / "pages"

    pages_dir.mkdir(parents=True, exist_ok=True)

    prototype_json_path = target / "prototype.json"
    index_page_path = pages_dir / "index.html"

    write_text(prototype_json_path, json.dumps(build_prototype_json(args.name), ensure_ascii=False, indent=2) + "\n")
    write_text(index_page_path, build_starter_page(args.name))

    print(f"Prototype initialized: {target}")


if __name__ == "__main__":
    main()
