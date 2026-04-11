---
name: ui-ux-pro-max
description: Use for UI/UX design, frontend polish, design systems, visual hierarchy, accessibility, motion, responsive layout, or interface reviews. Prefer this skill when the task changes how a screen looks, feels, or is interacted with. Skip it for backend-only, infra-only, or non-visual automation work.
---

# UI/UX Pro Max

This bundled skill vendors the MIT-licensed UI UX Pro Max dataset and search scripts so OpenAI Codex can use a strong UI-oriented skill out of the box.

## When to use

- Designing new screens, flows, dashboards, landing pages, admin panels, or mobile views
- Refactoring component structure, spacing, typography, color systems, motion, or responsive behavior
- Reviewing UI code for polish, usability, accessibility, or consistency
- Building or extending design systems and reusable component libraries

## Workflow

1. Resolve all paths relative to this skill directory.
2. Use the local search script before proposing a visual direction.
3. Generate a complete design system when the user asks for a new look, a major redesign, or a coherent multi-screen language.
4. Keep existing product conventions unless the user explicitly wants a re-theme.

## Common commands

```bash
python3 <skill_dir>/scripts/search.py "saas dashboard glassmorphism" --domain style
python3 <skill_dir>/scripts/search.py "settings form accessibility" --domain ux
python3 <skill_dir>/scripts/search.py "b2b analytics" --design-system -p "Analytics Console"
python3 <skill_dir>/scripts/search.py "table filtering dense data" --stack react
```

## Notes

- The searchable dataset covers styles, product categories, color systems, typography, charts, UX guidance, and stack-specific recommendations.
- Treat script output as structured reference material. Always reconcile it with the local codebase and the project's existing design system.
- Favor accessibility, interaction clarity, and platform-appropriate patterns over decorative effects.
