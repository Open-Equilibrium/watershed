import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import 'appearance.dart';
import 'controls.dart';
import 'settings_file.dart';
import 'shortcuts/keymap.dart';
import 'shortcuts/shortcut_store.dart';
import 'shortcuts/shortcuts_page.dart';
import 'theme/icons.dart';
import 'theme/theme.dart';
import 'theme/tokens.g.dart';

/// A section of Settings.
enum _Section { appearance, shortcuts, worktrees }

/// Settings: Appearance with theme and text size, Shortcuts with the
/// profile, every action's keys and recording new ones, and Worktrees with a session. It
/// replaces the main area; esc closes it, or first dismisses a failed save's notice.
class SettingsPage extends StatefulWidget {
  const SettingsPage({
    super.key,
    required this.shortcuts,
    required this.onClose,
    required this.saveFailure,
    required this.onDismissSaveFailure,
    this.appearance,
    this.worktrees,
  });

  final ShortcutController shortcuts;
  final VoidCallback onClose;

  /// The latest failed save of any settings, shown at the top until dismissed.
  final ValueListenable<SaveFailure?> saveFailure;
  final VoidCallback onDismissSaveFailure;

  /// Theme and text size; without them Settings show only Shortcuts.
  final AppearanceController? appearance;

  /// The Worktrees section; none without a session.
  final Widget? worktrees;

  @override
  State<SettingsPage> createState() => _SettingsPageState();
}

class _SettingsPageState extends State<SettingsPage> {
  final _focus = FocusNode(debugLabel: 'settings');

  /// The section shown; Appearance opens first.
  late _Section _section = widget.appearance != null
      ? _Section.appearance
      : _Section.shortcuts;

  @override
  void initState() {
    super.initState();
    // The shell keeps the focus otherwise, and esc would not reach the page.
    WidgetsBinding.instance.addPostFrameCallback((_) => _focus.requestFocus());
  }

  @override
  void dispose() {
    _focus.dispose();
    super.dispose();
  }

  /// esc dismisses the notice of a failed save while one shows, and closes Settings otherwise.
  void _escape() {
    if (widget.saveFailure.value != null) {
      widget.onDismissSaveFailure();
    } else {
      widget.onClose();
    }
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return CallbackShortcuts(
      bindings: {escapeKey: _escape},
      child: Focus(
        focusNode: _focus,
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Container(
              width: 150,
              padding: const EdgeInsets.all(WsMetrics.space4),
              decoration: BoxDecoration(
                color: ws.subtle,
                border: Border(right: BorderSide(color: ws.border)),
              ),
              // Hover and focus ink paints on a Material above the background.
              child: Material(
                type: MaterialType.transparency,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    Text(
                      'Settings',
                      style: TextStyle(
                        color: ws.textFaint,
                        fontSize: WsMetrics.fsSmall,
                      ),
                    ),
                    const SizedBox(height: WsMetrics.space2),
                    for (final (label, section) in [
                      if (widget.appearance != null)
                        ('Appearance', _Section.appearance),
                      ('Shortcuts', _Section.shortcuts),
                      if (widget.worktrees != null)
                        ('Worktrees', _Section.worktrees),
                    ])
                      Semantics(
                        selected: _section == section,
                        child: Shortcuts(
                          shortcuts: activateKeys,
                          child: InkWell(
                            onTap: () => setState(() => _section = section),
                            borderRadius: BorderRadius.circular(
                              WsMetrics.radiusMd,
                            ),
                            // Ink, so the highlights paint above the chosen section too.
                            child: Ink(
                              height: WsMetrics.rowNav,
                              padding: const EdgeInsets.symmetric(
                                horizontal: WsMetrics.space3,
                              ),
                              decoration: BoxDecoration(
                                color: _section == section ? ws.selected : null,
                                borderRadius: BorderRadius.circular(
                                  WsMetrics.radiusMd,
                                ),
                              ),
                              child: Align(
                                alignment: Alignment.centerLeft,
                                child: Text(
                                  label,
                                  style: const TextStyle(
                                    fontWeight: WsMetrics.fwMedium,
                                  ),
                                ),
                              ),
                            ),
                          ),
                        ),
                      ),
                  ],
                ),
              ),
            ),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  ValueListenableBuilder(
                    valueListenable: widget.saveFailure,
                    builder: (context, failure, _) => failure == null
                        ? const SizedBox.shrink()
                        : _SaveNotice(
                            text: failure.text,
                            onDismiss: widget.onDismissSaveFailure,
                          ),
                  ),
                  Expanded(
                    child: switch ((
                      _section,
                      widget.appearance,
                      widget.worktrees,
                    )) {
                      (_Section.appearance, final appearance?, _) =>
                        AppearancePane(appearance: appearance),
                      (_Section.worktrees, _, final worktrees?) => worktrees,
                      _ => ShortcutsPane(shortcuts: widget.shortcuts),
                    },
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// A failed save, in the danger colors, until ✕ dismisses it.
class _SaveNotice extends StatelessWidget {
  const _SaveNotice({required this.text, required this.onDismiss});

  final String text;
  final VoidCallback onDismiss;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Padding(
      padding: const EdgeInsets.fromLTRB(
        WsMetrics.space6,
        WsMetrics.space5,
        WsMetrics.space6,
        0,
      ),
      child: Container(
        padding: const EdgeInsets.symmetric(
          horizontal: WsMetrics.space3,
          vertical: WsMetrics.space2,
        ),
        decoration: BoxDecoration(
          color: ws.dangerSubtle,
          borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        ),
        child: Row(
          children: [
            Expanded(
              child: Text(
                text,
                style: TextStyle(
                  color: ws.danger,
                  fontSize: WsMetrics.fsSmall,
                  height: WsMetrics.lhSmall / WsMetrics.fsSmall,
                ),
              ),
            ),
            IconButton(
              tooltip: 'Dismiss',
              iconSize: 13,
              visualDensity: VisualDensity.compact,
              onPressed: onDismiss,
              icon: Icon(LucideIcons.x, color: ws.textFaint),
            ),
          ],
        ),
      ),
    );
  }
}
