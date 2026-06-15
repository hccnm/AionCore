# Functional-Test JSON Input

Functional-test JSON is optional. When present, treat it as a compact description of desired behavior and map it into the target project's `test-cases` case format when that contract is present.

## Read Order

1. Identify case names, feature names, user roles, preconditions, steps, expected results, and data requirements.
2. Match those details against real code in the selected directory.
3. Prefer the project's existing `test-cases` action vocabulary, mock/profile structure, and assertion syntax over the JSON's original vocabulary.
4. Preserve user intent, but rewrite it into directory-style case JSON.

## Guardrails

- Do not copy JSON action names into case files if the project has no matching runtime.
- Do not collapse a directory-style case project into one monolithic JSON file.
- Do not assume API paths or DOM selectors from JSON alone; verify them in code.
- If JSON describes behavior that is absent from the selected code, report the mismatch and ask for the correct target.

## Useful Mapping

| JSON Concept | Case JSON Target |
| --- | --- |
| scenario / caseName | `case[version].caseName.json` |
| role / actor | `mock-profile.json`, context key, auth setup node |
| precondition | `MOCK_SETUP` or setup node |
| operation step | `nodes[].action` |
| expected result | `nodes[].assertions[].expected` |
| cleanup | cleanup node or project teardown convention |
