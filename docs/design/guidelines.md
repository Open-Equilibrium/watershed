# Design guidelines

These rules apply to every Watershed GUI. Every GUI change starts with HTML mockups that the maintainer approves; approved mockups live in `mockups/` until their slice ships: the Workspace's [Pages](mockups/liquid-workspace-pages.html), [Blocks](mockups/liquid-workspace-blocks.html), [History](mockups/liquid-workspace-history.html) and [Sections](mockups/liquid-workspace-text-sections.html), the [Chats](mockups/liquid-chats.html) concept, [the app's structure](mockups/liquid-app-concept.html), and [pull requests and worktrees](mockups/liquid-git-prs-worktrees.html). Design tokens are canonical in [tokens.css](tokens.css); components use tokens, never raw values, and mockups share [mockup.css](mockups/mockup.css), which imports them. Flutter reads them from `liquid/app/lib/src/theme/tokens.g.dart`, which `scripts/design_tokens.py` generates; a tooling test fails when it falls out of date.

References: Notion, AppFlowy, Microsoft Loop and T3 Code.

## Principles

1. **Content first.** Chrome recedes: generous whitespace, few lines, controls appear when needed. Every screen still explains itself: a Chat's header names its participants and, for an agent, its work folder and model, and a new Chat greets with them; nothing is decorative.
2. **Calm neutrals, one accent.** Color always carries meaning: the accent marks selection and the primary action, warning marks "waiting for you", success and danger mark results.
3. **Dense where people develop.** Monospace for code, paths and tool calls; tabular figures for numbers; prose stays airy.
4. **Everything through tokens.** Color, type, spacing, radii, shadows and motion are named tokens, so changing a token changes every GUI.
5. **Light, dark and system are equal.** Text contrast meets WCAG AA; status never relies on color alone, always also on a symbol or text.
6. **Keyboard first.** Main actions and flows work without the mouse: the PowerBar (⌘K), a shortcut for every recurring action and a visible focus. Shortcuts stay out of the GUI: app-wide ones appear in the macOS menu bar, contextual ones in cards and dialogs appear only while the reveal key is held (⇪ with the Liquid key, otherwise ⌘), and Settings → Shortcuts lists and changes them. With the Liquid key on, every Liquid ⌘ shortcut also works as ⇪ plus the same key; platform standards (Settings…, Quit, Close, Hide, Minimize, clipboard, undo and text editing) stay ⌘ only.
7. **Stable layout.** Keep moving elements to a minimum: no decorative animation and no layout shift while content streams in; new content never moves what the user is reading or about to click.
8. **Same place, same thing.** Similar elements keep their position across views, including navigation, tabs, toolbars, the composer, the status bar and detail panes.
9. **Established words.** Name things as the established apps of the same domain do, such as Git's and T3 Code's "revert" for Git actions or Notion's terms for Pages; never invent a word for something users already know.

## Tokens

- **Color:** Notion's warm neutrals (`canvas`, `subtle`, `surface`, `hover`, `selected`), very light borders, three text levels, Liquid azure as the accent (the logo's blue, AA contrast), status colors (`success`, `warning`, `danger`), GitHub-style diff and syntax colors, data-series colors and ten tag colors for a select field's options, as Notion's (`tag-0` default to `tag-9` red, an option's stored color picking one and wrapping after ten; [mockup](mockups/liquid-database-table.html)); dark yellow is darker than Notion's so its text meets AA, which a token test checks for every tag (decided 2026-11-11). Surfaces are separated by background rather than borders. Light and dark values share names; a GUI selects `.theme-light` or `.theme-dark`, and system mode follows the operating system.
- **Type:** Notion's platform UI font stack and a monospace font for code, at Notion's sizes (ADR-0241): display 40/48 bold for Page titles, h1 30/38, h2 24/32 and h3 20/28 semibold, prose 16/24, UI 14/20, sidebar 14/20, compact 13/20 for dense rows, chips and buttons, small 12/18, caption 11/14 for chart axes, badges and tags, mono 13/20; the reading column is 708 px wide.
- **Spacing:** 4, 6, 8, 12, 16, 20, 24 and 32 px.
- **Radii:** 4 for tags, 6 for buttons, inputs and list items, 8 for cards and panels, 12 for windows, dialogs and the input area.
- **Rows:** 28 px for navigation items, 34 px for table rows, 45 px for headers.
- **Sidebar:** as dense as Notion's, AppFlowy's and Loop's: 240 px wide, one line per Page in 28 px rows; the chat list shows two lines per Chat, as WhatsApp does (ADR-0241).
- **Text size:** Settings → Appearance scales text and rows together: Compact, Default (the sizes above) or Large.
- **Motion:** 120 ms for hover and press, 200 ms for panels, ease-out; none when the user prefers reduced motion.
- **Icons:** Lucide (ISC), 16 px in rows and buttons, in the text color of their context; status icons always come with text (principle 5).
- **Logo:** [liquid-logo.svg](liquid-logo.svg) is the source of Liquid's app icon, on Apple's icon grid, and of its sidebar mark.

## Patterns

- The input area floats at the bottom of each tab's main column, centered within the reading width, as T3 Code's composer does; content runs on beneath it. Agent questions appear inside it, and widgets sit directly above it.
- Filters dim what does not match instead of hiding it; a range set with the timeline's handles limits the list to its steps. A click on the timeline or the session tree jumps to that step and moves a range that hides it there. The range handles are a two-thumb slider: Tab reaches the start, then the end handle; ← and → move one step, ⇧← and ⇧→ ten, Home and End go to the ends, ⌫ clears the range and esc leaves the handle. A handle shows its step while focused and is announced as a slider with it, the handles never cross, and a change announces the range, such as “Steps 12–31 of 48” (decided 2026-11-11).
- Below 760 px of window height a comparison's four metric cards become one line, so its first steps show without scrolling; taller windows keep the cards (decided 2026-11-11).
- A step's kind has one shape and color everywhere: the Kind tags, the timeline and the session tree.
- A choice of one among a few options uses the shared segments (`Segments` in `liquid/app/lib/src/controls.dart`): one framed control on the subtle background, the chosen segment raised on the surface, the same in every view; mockups that draw it otherwise are superseded.
- An action that cannot run stays visible, dimmed, and says why on hover.
- A row's menu button (···) sits inside the row at its right end, seen on hover or keyboard focus; a session card shows it in place of its status word. Right-click opens the same menu.
- A session's menu lists “Export record” above “Delete session…”. It writes the session's record folder to Downloads and shows it in Finder, without a save dialog, since Finder lets the person move the folder; the status bar says where it went (decided 2026-11-11).
- Results and errors appear as notices in the status bar, Liquid's place for messages, not as toasts: errors stay until esc or a click, info and warnings fade after 6 s, and a click lists the latest ones with their whole text. While Settings are open, which cover the status bar, a failed save shows at their top instead, until ✕, esc or the next save of the same settings.
- An error starts with a plain sentence that says what failed and whether anything changed, then quotes the tool's own message, with credentials removed.
- Liquid's shared question dialog puts Cancel on the left and its main action on the right, its other actions between them, as macOS and Material place them; older mockups list the main action first in their markup and show it this way ([decided](mockups/liquid-database-table.html) on 2026-11-08).
- Automations, History, Page history and Settings open as a dialog over the current Page or Source, and closing it shows that Page again; the Settings page that replaced the main area in older mockups is superseded.
- A pick from a remote's list, such as a pull request's base branch, shows the list's first page and also searches the remote as the person types, after a short pause, saying while it searches. A typed name the remote does not know is refused; when the search fails the first page stays and Enter takes a typed name as before, checked when the plan is read again (decided 2026-11-11).
