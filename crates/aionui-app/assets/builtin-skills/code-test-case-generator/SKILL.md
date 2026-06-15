---
name: code-test-case-generator
description: Generate directory-style functional test-case JSON files from an existing codebase, with optional functional-test JSON as input. Use for creating test-cases/<system>/<feature>/case[version].caseName.json files that match the target project's executor.
---

# Code Test Case Generator

Generate functional test-case files for the selected project. The default output is source-controlled `test-cases` JSON files in the target repository, not product docs, PRDs, prototypes, or ad hoc unit-test classes.

This skill is intentionally biased toward the `test-cases/<system>/<feature>/case[version].caseName.json` contract. Only generate framework-native code tests when the user explicitly asks for code tests, or when the selected project has no `test-cases` contract and no JSON case executor.

## Inputs

Use any available input, in this order:

1. User-selected workspace, attached folder, or explicit path.
2. Optional functional-test JSON uploaded or pasted by the user.
3. Existing `test-cases/`, `test-histories/`, `mock-profile.json`, case reports, and case executor scripts.
4. Existing source code, routes, services, components, schemas, fixtures, mocks, and tests.
5. Project manifests such as `package.json`, `pom.xml`, `Cargo.toml`, `pyproject.toml`, `pytest.ini`, `playwright.config.*`, `vitest.config.*`, or `jest.config.*`.

If no JSON is provided, infer functional cases from code and nearby existing `test-cases`. Do not ask for JSON as a prerequisite.

## Workflow

1. Identify the project type, `test-cases` layout, case naming style, mock/profile pattern, executor command, and target feature boundary.
2. Read at least one existing case file from the same or closest feature before writing new cases.
3. Read the target feature code and verify every route, method, DTO field, enum, status, permission branch, and response field used by the case.
4. If functional-test JSON exists, map each scenario into the project's `test-cases` JSON format.
5. If no JSON exists, derive scenarios from observable behavior, public interfaces, route handlers, UI flows, service methods, validation rules, state transitions, and uncovered edge cases.
6. Write focused `case[version].caseName.json` files in the existing style, plus `mock-profile.json` only when needed.
7. Hand off to `code-test-runner` to execute the smallest relevant case command or JSON validation.

## Generation Rules

- Prefer existing `test-cases` examples, mock profiles, action vocabularies, assertion formats, context keys, fixtures, and setup utilities.
- Keep the cases close to the target feature's current case style.
- Test observable behavior, not private implementation details.
- Include at least one failure or edge path when the feature has meaningful validation or error handling.
- Use graph-capable fields: top-level `desc`, `nodes`, and `edges`; use `waiting.nodeIds` when a step waits for another branch instead of encoding it as a normal edge.
- Use `expected`, not misspelled variants such as `exprected`.
- Use expression-style context keys such as `{{<adminContext>id}}` and `{{<playerContext>user.name}}` when setting or reading context.
- Do not invent routes, selectors, API fields, test accounts, environment variables, helper functions, status values, enum names, or assertion fields.
- If a required fact cannot be discovered from the project, ask only the blocking question.

## Output Contract

When `test-cases` mode applies, write files like:

```text
test-cases/<system>/<feature>/
  mock-profile.json
  case[1.0.0].happyPathName.json
  case[1.0.0].edgeCaseName.json
```

Each case file must be a standalone JSON object:

- `desc`: concise business/engineering behavior summary.
- `nodes`: ordered or graph-addressable steps, each with stable `id` values such as `n0`, `n1`.
- `nodes[].action`: executor action, usually `MOCK_SETUP`, `API_CALL`, UI action, or another action type already present in existing cases.
- `nodes[].assertions`: assertions using `exp` plus `expected`.
- `nodes[].setContext`: context writes using the project's expression syntax when downstream nodes need data.
- `edges`: graph links with stable ids such as `e1`.

## JSON Mapping

Functional-test JSON is scenario input. In `test-cases` mode, it is mapped into the target project's case JSON files.

When JSON exists, map it as follows:

- Case or scene name -> `case[version].caseName.json`.
- Preconditions -> `MOCK_SETUP`, `mock-profile.json`, seeded context, or setup nodes.
- Steps -> `nodes` with verified API calls, UI actions, service invocations, or integration flow actions.
- Assertions -> `assertions[].expected`.
- Cleanup -> cleanup node or existing teardown case convention.

See `references/json-input.md`, `references/test-cases-project.md`, and `templates/test-plan.md` for the expected analysis shape.
