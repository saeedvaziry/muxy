---
name: code-review
description: Muxy-tailored code review. Use when the user asks to review a PR, the current branch against a target branch, or the current changes on the branch.
---

# Code Review

Single-pass, read-only review tailored to Muxy. Gather evidence once and assess specification achievement and repository standards from that same evidence. Do not build, lint, run tests, or run `scripts/checks.sh` unless the user explicitly asks. Never post to GitHub. Recommend fixes in the report and apply them only after the user confirms.

Do not spawn sub agents for review.

## Determine the target

- PR number given: fetch the title, description, changed files, linked issue references, and diff with `gh`. Fetch linked issue contents only when a reference exists. Batch independent `gh` reads when practical. The PR and linked issue define the spec.
- Target branch given: diff the current branch against it. The spec comes from the stated task or the branch commits.
- Nothing given: review the working tree changes; if the tree is clean, review the branch commits against `main`.
- If the user states the task or spec directly, review against that instead.

## Investigation scope

Start from the changed hunks. Inspect the enclosing declarations, directly affected callers or callees, and the closest relevant tests. Apply repository standards as filters where they are relevant to the changed behavior; do not perform a repository-wide audit for every checklist item.

For a small change of at most 5 files and 200 added or deleted lines, use this fast path:

1. Fetch metadata and the diff in one tool round when practical.
2. Read changed code, direct references, and relevant tests in one batched context pass.
3. Use one focused follow-up pass only when the evidence exposes a concrete ambiguity or risk.

For larger changes, expand proportionally to risk. Search beyond direct dependencies only when a concrete finding cannot otherwise be confirmed. Do not use open-ended exploration to seek certainty. If evidence remains unavailable, state the limitation instead of guessing.

Do not proactively hunt for unrelated issues. Report an unrelated problem only when it is encountered while following a changed code path. Read history or documentation only when the specification is ambiguous or the changed behavior affects documented behavior, public APIs, configuration, workflows, or extension contracts.

Stop investigating when every changed behavior has been mapped to the specification, its direct interactions and relevant tests have been checked, and no concrete unresolved risk remains.

## Review order

1. **Spec achievement** — verify the change fully and properly accomplishes the PR/issue/task. Call out gaps, missed requirements, and scope creep.
2. **Repo standards** — review the applicable repository rules, with emphasis on:
   - Security and correctness
   - Maintainability and architecture when responsibilities or abstractions change
   - Root-cause fixes rather than symptom patches
   - No code comments; self-explanatory code; early returns
   - Memory, CPU, Swift 6 concurrency, and SwiftUI correctness when those concerns are touched
   - Tests cover critical paths without overtesting
   - Documentation accuracy when documented behavior changes; extension API changes must update the extension SKILL and docs
   - AI-contributed PRs must name the LLM in the description

## Report

- **TL;DR** — verdict and what the user needs to decide, first.
- **Spec assessment** — achieved / partially achieved / not achieved, with specifics.
- **Changes needed** — findings with severity (Critical / High / Medium / Low), each with a concrete recommended fix.
- **Nits & advisory** — optional improvements, separated from required changes.
- **Unrelated issues** — include only when a problem was encountered incidentally; otherwise omit this section.

Report only actionable findings supported by evidence introduced by the reviewed change. Omit empty optional sections.
