# Design guidelines

These rules apply to every Watershed GUI (ADR-0194). Every GUI change starts with HTML mockups that the maintainer approves; approved mockups are stored next to this file. Design tokens are canonical in [tokens.css](tokens.css); components use tokens, never raw values.

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

- **Color:** neutral surfaces (`canvas`, `subtle`, `surface`, `hover`, `selected`), borders, three text levels, one accent, status colors (`success`, `warning`, `danger`), diff, syntax and data-series colors. Light and dark values share names; a GUI selects `.theme-light` or `.theme-dark`, and system mode follows the operating system.
- **Type:** the platform UI font and a monospace font for code. Scale: title 28/600, h2 20/600, h3 16/600, prose 15 with line height 1.6, UI 13, small 12, caption 11, mono 12.
- **Spacing:** a 4 px grid (4, 8, 12, 16, 24, 32).
- **Radii:** 4 for chips and inputs, 8 for cards, 12 for windows and dialogs.
- **Motion:** 120 ms for hover and press, 200 ms for panels, ease-out; none when the user prefers reduced motion.
