---
name: test-discovery-rules
description: Discover meaningful functional test-case targets from project code when no functional-test JSON is provided. Use before generating test-cases JSON from a selected code directory.
---

# Test Discovery Rules

Discover what functional cases to generate from the selected code directory. This skill is for engineering test-case work only.

## Discovery Sources

Inspect the project in this order:

1. Existing `test-cases/`, `test-histories/`, `mock-profile.json`, case README files, case reports, and case execution scripts.
2. Manifests and configs that reveal language, framework, package manager, and test or case commands.
3. Existing code tests nearest to the target feature.
4. Public entry points: routes, controllers, API clients, components, hooks, services, jobs, command handlers, schemas, or validators.
5. Fixtures, factories, mocks, seed data, auth helpers, and setup utilities.
6. Error handling, boundary conditions, validation rules, permissions, state transitions, and regression-prone branches.

## Scenario Selection

When no functional-test JSON is provided, choose case scenarios that are grounded in code:

- Happy path for the public behavior.
- Validation or error path when present.
- Permission, state transition, or role branch when explicit in code.
- Regression case for an existing bug marker, TODO, issue reference, or nearby fragile branch.

Avoid creating large speculative suites. A small, high-signal case set that matches project style is better than broad invented coverage.

## Evidence Rules

- Every route, selector, DTO field, status, enum, helper, fixture, and command must be backed by code evidence.
- If a behavior is implied but not implemented, state that and ask before testing it.
- If multiple case formats or test frameworks exist, pick the one with the closest existing coverage for the target feature.
- If no `test-cases` contract or executor is discoverable, ask whether to generate directory-style case JSON or framework-native code tests before adding a new format.

See `references/discovery-checklist.md` for a quick checklist.
