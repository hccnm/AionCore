# Discovery Checklist

Use this checklist before writing tests from code alone.

## Project Shape

- Language and package manager identified.
- Test framework identified.
- Existing test directory and naming style identified.
- Smallest relevant test command identified.

## Target Shape

- Public entry point identified.
- Existing behavior traced to source code.
- Nearby helpers, mocks, and fixtures identified.
- Required auth, env, DB, browser, or external service assumptions identified.

## Scenario Quality

- At least one high-value behavior selected.
- Failure or edge path included when the code exposes one.
- Assertions target observable behavior.
- No unverified route, selector, field, helper, or env var is used.

## Stop And Ask

Ask a concise blocking question when:

- The target directory is missing or ambiguous.
- No test framework is discoverable.
- Required credentials or environment are necessary to run the test.
- The requested behavior is not present in the selected code.
