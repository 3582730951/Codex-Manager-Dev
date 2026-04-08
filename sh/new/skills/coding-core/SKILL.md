---
name: coding-core
description: Unified coding workflow for implementation, debugging, refactors, testing, APIs, data changes, and deploy-safe edits.
---

# Coding Core

Use this skill when the task is primarily software engineering work and you want one consistent workflow instead of a large vendor skill bundle.

## Goals

- Keep the loop short: inspect, change, verify, summarize.
- Make the smallest change that fully solves the task.
- Preserve existing architecture and naming unless there is a clear defect.
- Prefer reproducible commands and explicit rollback paths.

## Default Workflow

1. Read the relevant files first and identify the true entrypoints.
2. State the assumption that would most likely invalidate the change if wrong.
3. Implement the narrowest patch that fixes the problem.
4. Run the closest verification available: tests, typecheck, lint, build, smoke, or focused curl checks.
5. Report the outcome, residual risk, and any skipped validation.

## When Implementing

- Trace input shape, validation, core logic, persistence, and outward response before editing.
- Keep interfaces backward compatible unless the task explicitly allows a breaking change.
- Prefer data-shape normalization at boundaries.
- Avoid broad rewrites when a local fix is sufficient.

## When Debugging

- Reproduce first.
- Compare expected vs actual behavior with one concrete request or fixture.
- Check logs, error handling, environment assumptions, and stale state before rewriting logic.
- If the issue may be timing-related, verify with a second run after the fix.

## APIs And Schemas

- Update validators, handlers, persistence, and tests together.
- Be explicit about defaults, nullable fields, and migration behavior.
- Preserve wire compatibility when downstream clients may already depend on it.

## Frontend Work

- Preserve the product's existing design language unless redesign is requested.
- Fix state/data flow first, then polish layout and copy.
- Verify empty, loading, error, and success states.

## Refactors

- Separate mechanical moves from behavior changes when possible.
- Keep public contracts stable.
- Prove no regression with a focused test or smoke path.

## Verification Ladder

- Small logic fix: run the smallest direct test that exercises the changed path.
- Cross-file change: add or run at least one integration-level check.
- Deploy/runtime change: include a live command that proves the service is up.

## Output Standard

- What changed.
- How it was verified.
- What remains uncertain.
