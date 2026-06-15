# Code Test Case Agent Skills

This built-in agent enables the following engineering test skills by default:

- `test-discovery-rules`: discover feature boundaries, APIs, state transitions, existing `test-cases` examples, and coverage gaps from a code directory.
- `code-test-case-generator`: generate directory-style functional cases such as `test-cases/<system>/<feature>/case[version].caseName.json` from optional functional-test JSON or project code.
- `code-test-runner`: choose an existing case executor or the smallest verification command, run generated cases, and repair case failures from logs.

These skills serve only the engineering test loop: read code, write functional test cases, run cases or minimal verification, and repair cases.
