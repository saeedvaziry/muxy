---
name: implement
description: Implement a user-requested feature or bugfix, including tests, exhaustive diff review, verified fixes, and a bounded completion decision.
---

# Implement

## Define

- Read the provided context, repository instructions, and relevant code.
- Clarify the gaps that materially affect the result.
- Agree on acceptance criteria, out-of-scope work, and the exact diff base.
- Ask user questions with providing options if possible for more clarify if needed.

## Plan

- Make a proportional implementation plan.
- For substantial work, present it with `plan-presenter` when available and wait for approval.

## Build

- Implement the change that satisfies the acceptance criteria and repository standards.
- Add focused tests.
- Run the relevant checks before review.
- Refactorings are not forbidden, you just need to bring it to the attention of the user for confirmation.

## Review

- Spawn a sub-agent to review the code using project's `code-review` skill (if available).
- Verify the findings

## Fix and verify

- If the review passes, Finish and report.
- Apply a fix to the verified findings but you can skip the advisories on your judgment.
- Run the checks and fix the appearing issues and inform user if so.
- Do not review again. the cycle must happen only once.

## Finish

- Report with a short sumamry
- If the feature/fix is testable, provide guides to the user to test
