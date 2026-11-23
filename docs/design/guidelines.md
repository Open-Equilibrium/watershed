# Design guidelines

These rules apply to every Watershed GUI. Every GUI change starts with HTML mockups that the maintainer approves, except Liquid's agent Chat and agent hosts settings, which copy T3 Code's newest desktop app (ADR-0295); approved mockups live in `mockups/` until their slice ships: the Workspace's [Pages](mockups/liquid-workspace-pages.html), [Blocks](mockups/liquid-workspace-blocks.html), [History](mockups/liquid-workspace-history.html), [Sections](mockups/liquid-workspace-text-sections.html) and [text formatting](mockups/liquid-text-formatting.html), the [Chats](mockups/liquid-chats.html) concept and [the app's structure](mockups/liquid-app-concept.html). Design tokens are canonical in [tokens.css](tokens.css); components use tokens, never raw values, and mockups share [mockup.css](mockups/mockup.css), which imports them. Flutter reads them from `liquid/app/lib/src/theme/tokens.g.dart`, which `scripts/design_tokens.py` generates; a tooling test fails when it falls out of date.

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

- **Color:** Notion's warm neutrals (`canvas`, `subtle`, `surface`, `hover`, `selected`), very light borders, three text levels, Liquid azure as the accent (the logo's blue, AA contrast), status colors (`success`, `warning`, `danger`), GitHub-style diff and syntax colors, data-series colors and ten tag colors for a select field's options, as Notion's (`tag-0` default to `tag-9` red, an option's stored color picking one and wrapping after ten; a new option takes the next); dark yellow is darker than Notion's so its text meets AA, which a token test checks for every tag. Surfaces are separated by background rather than borders. Light and dark values share names; a GUI selects `.theme-light` or `.theme-dark`, and system mode follows the operating system.
- **Type:** Notion's platform UI font stack and a monospace font for code, at Notion's sizes (ADR-0241).
- **Layout:** spacing, radii, row heights, sidebar and reading widths are tokens. The sidebar is as dense as Notion's, AppFlowy's and Loop's: one line per Page, and two lines per Chat, as WhatsApp does (ADR-0241).
- **Text size:** Settings → Appearance scales text and rows together: Compact, Default (the token sizes) or Large.
- **Motion:** short transitions for hover and press, longer ones for panels, ease-out; none when the user prefers reduced motion.
- **Icons:** Lucide (ISC), 16 px in rows and buttons, in the text color of their context; status icons always come with text (principle 5).
- **Logo:** [liquid-logo.svg](liquid-logo.svg) is the source of Liquid's app icon, on Apple's icon grid, and of its sidebar mark.

## Patterns

- The input area floats at the bottom of each tab's main column, centered within the reading width, as T3 Code's composer does; content runs on beneath it. Agent questions appear inside it, and widgets sit directly above it.
- An agent's task list sits in the Chat where T3 Code puts it, as one row, "Tasks 2/5 · Explore", that opens into its steps. Steps named `Phase › instance` hang under their Phase as a tree. Hovering a running instance shows Message, which starts the composer with `@instance `. Typing `@` as the composer's first character lists the running instances, as T3 lists files, and Enter or Tab inserts one (ADR-0314).
- Persistent agents (ADR-0283) sit in an Agents section above Threads, which no longer lists their threads; a row shows the agent's state word, its next run or an unread dot. + opens an empty agent Chat with starters, and the first message is the job. In an agent's Chat, each change the agent made to itself follows its turn as a line with Open, Run now or Undo; a routine's run is headed by the routine's name and time; the Chat opens at “Since your last visit”; an earlier thread is read-only. The session card's Agent section edits name, model, permissions and instructions, and its Automations add routines. `@agent` in another Chat sends the message to that agent and shows its answer there, below the person's message. An agent's row menu offers Move to host…, the session move dialog with what moves along: its memory folder and size, the current thread, its routines, and the earlier threads that stay.
- A choice of one among a few options uses the shared segments (`Segments` in `liquid/app/lib/src/controls.dart`): one framed control on the subtle background, the chosen segment raised on the surface, the same in every view; mockups that draw it otherwise are superseded.
- An action that cannot run stays visible, dimmed, and says why on hover.
- Where a click on a column's name sorts, the column's menu opens from ⌄ beside the name, seen on hover and always on touch, or by a right-click or long press.
- A row's menu button (···) sits inside the row at its right end, seen on hover or keyboard focus; the sidebar's session row shows it in place of its status word. Right-click opens the same menu.
- A dialog row that does not fit offers its fixes as segments in a row of their own below its text, and the chosen fixes run with the dialog's main action, which stays available; Liquid's “Move to host” dialog is the model (ADR-0308).
- A session's menu lists “Export record” above “Delete session…”. It writes the session's record folder to Downloads and shows it in Finder, without a save dialog, since Finder lets the person move the folder; the status bar says where it went.
- Results and errors appear as notices in the status bar, Liquid's place for messages, not as toasts: errors stay until esc or a click, info and warnings fade after 6 s, and a click lists the latest ones with their whole text. While Settings are open, which cover the status bar, a failed save shows at their top instead, until ✕, esc or the next save of the same settings.
- An error starts with a plain sentence that says what failed and whether anything changed, then quotes the tool's own message, with credentials removed.
- Liquid's shared question dialog puts Cancel on the left and its main action on the right, its other actions between them, as macOS and Material place them.
- Automations, History, Page history and Settings open as a dialog over the current Page or Source, and closing it shows that Page again.
- A pick from a remote's list, such as a pull request's base branch, shows the list's first page and also searches the remote as the person types, after a short pause, saying while it searches. A typed name the remote does not know is refused; when the search fails the first page stays and Enter takes a typed name as before, checked when the plan is read again.
