# Directory-Style Test-Cases Project Contract

Use this contract whenever the user asks for `test-cases`, the selected project already contains a `test-cases/` tree, or functional-test JSON is meant to become executable functional cases.

## Directory Layout

```text
test-cases/
  <systemName>/
    <featureName>/
      mock-profile.json
      case[1.0.0].caseName.json
      case[1.0.0].edgeCaseName.json
```

`json_output_path` may be a legacy name in prompts or upstream JSON. In this context it means a directory-style case project path, not one monolithic JSON file.

## Case File Shape

Each `case[version].caseName.json` file is a standalone JSON object with graph-capable fields:

- `desc`: concise summary of the behavior under test.
- `nodes`: executable steps with stable ids such as `n0`, `n1`, `n2`.
- `nodes[].workerIndex`: use the same actor/index convention as nearby cases.
- `nodes[].desc`: human-readable step description.
- `nodes[].action.type`: action vocabulary from existing cases, such as `MOCK_SETUP` or `API_CALL`.
- `nodes[].action.value`: route/action target verified from project code.
- `nodes[].action.args`: request body, query object, mock key, or action args.
- `nodes[].action.timeout`: timeout aligned with nearby cases.
- `nodes[].assertions`: assertions with `desc`, `exp`, and `expected`.
- `nodes[].setContext`: optional context writes using expression keys such as `{{<adminContext>id}}`.
- `nodes[].waiting.nodeIds`: optional wait references for branch synchronization.
- `edges`: graph links with stable ids such as `e1`, `source`, and `target`.

Use `expected`; do not emit misspelled keys such as `exprected`.

## Mock Profile

Create or update `mock-profile.json` only when the case needs mock data. Keep it in the same feature directory as the cases.

Typical fields:

- `desc`
- `profile`
- `config`
- `mockData`

Do not invent accounts, tokens, ids, feature flags, URLs, or config keys unless code or existing profiles support them.

## Evidence Requirements

Before writing a case, verify every case-facing fact against code or nearby cases:

- API method and path.
- Request body fields and query params.
- Response fields used in assertions.
- Status codes, enum values, and state names.
- Auth or actor context.
- Mock keys and profile fields.
- Executor action names and assertion syntax.

If a required route or field cannot be verified, ask the blocking question or write a clearly marked TODO only if the project already uses TODO placeholders in cases.

## When Not To Write Code Tests

Do not create `src/test/java`, `*.spec.ts`, `*_test.py`, or similar framework-native tests when any of these is true:

- The user asked for `test-cases`.
- The project has an existing `test-cases/` tree for the target module.
- The provided functional-test JSON is intended for the directory-style executor.
- Nearby examples follow the `case[version].caseName.json` format.

Generate framework-native code tests only when the user explicitly asks for code tests or the project has no directory-style case contract.
