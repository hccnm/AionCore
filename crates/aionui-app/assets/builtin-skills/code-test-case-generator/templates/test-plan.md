# Test Generation Plan

Use this compact structure internally before writing files.

```markdown
## Target
- Directory:
- Feature:
- Existing test-cases layout:
- Existing nearest case:
- Existing mock profile:
- Smallest case/validation command:

## Inputs
- Functional JSON: present / absent
- User-stated target:
- Code evidence:

## Cases To Add
- Case file:
- File path:
- Mock/profile changes:
- Nodes:
- Edges:
- Assertions:
- Why this is valuable:

## Unknowns
- Blocking:
- Non-blocking assumptions:
```

Only expose this plan to the user when it helps clarify a risky choice. Otherwise use it as working notes and proceed to implementation.
