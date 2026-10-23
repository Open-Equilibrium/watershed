# Design guidelines

These rules apply to every Watershed GUI (ADR-0194). Every GUI change starts with HTML mockups that the maintainer approves; approved mockups live in [mockups/](mockups/liquid-agent-surface.html). Design tokens are canonical in [tokens.css](tokens.css); components use tokens, never raw values.

References: Notion, AppFlowy, Microsoft Loop and T3 Code.

## Principles

1. **Content first.** Chrome recedes: generous whitespace, few lines, controls appear when needed.
2. **Calm neutrals, one accent.** Color always carries meaning: the accent marks selection and the primary action, warning marks "waiting for you", success and danger mark results.
3. **Dense where people develop.** Monospace for code, paths and tool calls; tabular figures for numbers; prose stays airy.
4. **Everything through tokens.** Color, type, spacing, radii, shadows and motion are named tokens, so changing a token changes every GUI.
5. **Light, dark and system are equal.** Text contrast meets WCAG AA; status never relies on color alone, always also on a symbol or text.
6. **Keyboard first.** Main actions and flows work without the mouse: the PowerBar (⌘K), a shortcut for every recurring action and a visible focus.
7. **Stable layout.** Keep moving elements to a minimum: no decorative animation and no layout shift while content streams in; new content never moves what the user is reading or about to click.
8. **Same place, same thing.** Similar elements keep their position across views, including navigation, tabs, toolbars, the composer, the status bar and detail panes.

## Tokens

- **Color:** Notion's warm neutrals (`canvas`, `subtle`, `surface`, `hover`, `selected`), very light borders, three text levels, Notion blue as the accent (darkened for AA contrast), status colors (`success`, `warning`, `danger`), GitHub-style diff and syntax colors and data-series colors. Surfaces are separated by background rather than borders. Light and dark values share names; a GUI selects `.theme-light` or `.theme-dark`, and system mode follows the operating system.
- **Type:** Notion's platform UI font stack and a monospace font for code, with AppFlowy's sizes: display 32/40 bold, h1 24/32, h2 20/28 and h3 16/22 semibold, prose 16/24, UI 14/20, small 12/18, mono 13/20.
- **Spacing:** 4, 6, 8, 12, 16, 20, 24 and 32 px.
- **Radii:** 4 for tags, 6 for buttons, inputs and list items, 8 for cards and panels, 12 for windows, dialogs and the input area.
- **Rows:** 28 px for navigation items, 34 px for table rows.
- **Motion:** 120 ms for hover and press, 200 ms for panels, ease-out; none when the user prefers reduced motion.

## Patterns

- The input area keeps one position and height in every view of a session; approvals and agent dialogs appear inside it, and widgets sit directly above it.
- Focusing a range dims everything outside it instead of filtering it out.
