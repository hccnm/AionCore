---
name: code-test-runner
description: Detect and run the smallest relevant case executor or verification command after generating test-cases JSON, then repair failures using the output. Use after adding or changing functional test cases.
---

# Code Test Runner

Run the generated or related functional cases and use the output to repair case JSON when possible.

## Command Discovery

Find the smallest useful command from project evidence:

- Case projects: README files, `test-cases/*`, `test-histories/*`, scripts that mention case execution, JSON validation, or report generation.
- JavaScript or TypeScript: `package.json`, lockfiles, `vitest.config.*`, `jest.config.*`, `playwright.config.*`.
- Java or Kotlin: `pom.xml`, `build.gradle`, existing `*Test` classes.
- Python: `pyproject.toml`, `pytest.ini`, `tox.ini`, existing `test_*.py`.
- Rust: `Cargo.toml`, existing `#[test]` or integration tests.
- Go: `go.mod`, existing `*_test.go`.

Prefer targeted commands over full-suite commands, such as a single case directory, case file, module, class, package, or test name filter.

## Execution Rules

- Run verification only after generating or changing case files or test code.
- If the project has a case executor, run the smallest relevant case command.
- If no case executor is discoverable, at least validate the new JSON files with the platform's standard JSON tooling and check referenced local files exist.
- Use the project's package manager and wrapper when visible, such as `bun`, `pnpm`, `npm`, `mvnw`, `gradlew`, or `cargo`.
- If dependencies are missing or the command needs external network access, report the blocker instead of inventing a result.
- If a failure is caused by the new case JSON, mock profile, or generated test code, fix it and rerun the targeted command.
- If a failure is caused by existing project behavior or missing external services, report it clearly with the command and failure summary.

## Repair Loop

Limit self-repair to focused fixes:

1. Read the failing stack trace and failing assertion.
2. Compare the case with nearby passing cases or reports.
3. Fix action names, paths, args, context expressions, assertion fields, fixture setup, selectors, imports, helper usage, async waits, or expected values only when verified by code.
4. Rerun the same targeted command.

Stop when the case passes, JSON validation passes with no executor available, the failure is external, or further changes would alter production behavior outside the user's request.

See `references/test-commands.md` for command selection examples.
