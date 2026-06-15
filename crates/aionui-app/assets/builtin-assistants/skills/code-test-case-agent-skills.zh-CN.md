# 代码测试用例 Agent 技能说明

这个内置 Agent 默认启用以下工程测试技能：

- `test-discovery-rules`：从代码目录发现功能边界、接口、状态流转、现有 `test-cases` 样例和覆盖缺口。
- `code-test-case-generator`：基于功能测试 JSON 或项目代码生成 `test-cases/<system>/<feature>/case[version].caseName.json` 目录式功能测试用例。
- `code-test-runner`：选择项目已有 case 执行器或最小验证命令，运行新增 case，并根据失败日志修复用例。

这些技能只服务工程测试闭环：读代码、写功能测试用例、跑用例或最小验证、修用例。
