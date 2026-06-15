# Test Command Selection

Choose the narrowest command that proves the generated case or test is discoverable and meaningful.

## Directory-Style Test-Cases

- Prefer the project's documented case executor from README, scripts, or nearby reports.
- Prefer a single feature directory such as `test-cases/<system>/<feature>` over the full case suite.
- Prefer a single case file when the executor supports it.
- If no executor is discoverable, validate generated JSON files with a standard JSON parser and check referenced local files such as `mock-profile.json` exist.

## JavaScript / TypeScript

- Prefer `bun run test -- path/to/file.test.ts` when Bun and Vitest are used.
- Prefer `pnpm test -- path/to/file.test.ts` when pnpm is the project package manager.
- Use `npx playwright test path/to/spec.ts` only when Playwright is the visible framework.

## Java / Maven

- Prefer `./mvnw -q -Dtest=ClassName test` when a wrapper exists.
- Otherwise use `mvn -q -Dtest=ClassName test` if the project already uses Maven.
- For multi-module projects, include the module selector when existing commands show it.

## Python

- Prefer `pytest path/to/test_file.py -q`.
- Add a test name filter only after the full file command is too broad or slow.

## Rust

- Prefer `cargo test test_name` for unit tests.
- For package workspaces, include `-p package_name` when the target package is clear.

## Reporting

Always report:

- Exact command.
- Pass or fail.
- Failure summary if any.
- Whether failures came from generated cases/tests, existing cases/tests, or external setup.
