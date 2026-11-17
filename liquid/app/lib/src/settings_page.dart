import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import 'agent/session_card_settings.dart';
import 'agent/start_page.dart';
import 'appearance.dart';
import 'controls.dart';
import 'rust/api/session.dart';
import 'session/backend.dart';
import 'settings_file.dart';
import 'shortcuts/keymap.dart';
import 'shortcuts/shortcut_store.dart';
import 'shortcuts/shortcuts_page.dart';
import 'theme/icons.dart';
import 'theme/theme.dart';
import 'theme/tokens.g.dart';

/// A section of Settings.
enum _Section { appearance, shortcuts, writer, agentHosts }

/// Settings: Appearance with theme, text size and thread details, Shortcuts with the profile, every
/// action's keys and recording new ones, the Writer model with a project, and Agent hosts. It
/// replaces the main area; esc closes it, or first dismisses a failed save's notice.
class SettingsPage extends StatefulWidget {
  const SettingsPage({
    super.key,
    required this.shortcuts,
    required this.onClose,
    required this.saveFailure,
    required this.onDismissSaveFailure,
    this.appearance,
    this.cardSettings,
    this.writer,
    this.agentHosts,
  });

  final ShortcutController shortcuts;
  final VoidCallback onClose;

  /// The latest failed save of any settings, shown at the top until dismissed.
  final ValueListenable<SaveFailure?> saveFailure;
  final VoidCallback onDismissSaveFailure;

  /// Theme and text size; without them Settings show only Shortcuts.
  final AppearanceController? appearance;

  /// The session card's rows, which Appearance sets as Thread details.
  final SessionCardSettingsController? cardSettings;

  /// The Writer model section; none without the project's backend.
  final Widget? writer;

  /// The Agent hosts section (ADR-0295); none without the agent hosts' settings.
  final Widget? agentHosts;

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
                      if (widget.writer != null)
                        ('Writer model', _Section.writer),
                      if (widget.agentHosts != null)
                        ('Agent hosts', _Section.agentHosts),
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
                      widget.writer,
                      widget.agentHosts,
                    )) {
                      (_Section.appearance, final appearance?, _, _) =>
                        AppearancePane(
                          appearance: appearance,
                          cardSettings: widget.cardSettings,
                        ),
                      (_Section.writer, _, final writer?, _) => writer,
                      (_Section.agentHosts, _, _, final hosts?) => hosts,
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

/// The writer model (ADR-0294): the one model that writes commit messages left blank, pull request
/// titles and descriptions, and session titles, read and set through the project's backend.
class WriterModelPane extends StatefulWidget {
  const WriterModelPane({super.key, required this.project});

  final ProjectBackend project;

  @override
  State<WriterModelPane> createState() => _WriterModelPaneState();
}

class _WriterModelPaneState extends State<WriterModelPane> {
  List<BridgeProvider>? _providers;
  BridgeModelChoice? _choice;
  String? _problem;

  @override
  void initState() {
    super.initState();
    unawaited(_read());
  }

  Future<void> _read() async {
    try {
      final providers = await widget.project.providers();
      final choice = await widget.project.writerModel();
      if (!mounted) return;
      setState(() {
        _providers = providers;
        _choice = choice;
      });
    } on Object catch (error) {
      if (mounted) {
        setState(() => _problem = 'Could not read the writer model: $error');
      }
    }
  }

  Future<void> _set(BridgeModelChoice next) async {
    setState(() {
      _choice = next;
      _problem = null;
    });
    try {
      await widget.project.setWriterModel(next);
    } on Object catch (error) {
      if (mounted) {
        setState(() => _problem = 'Could not save the writer model: $error');
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Padding(
      padding: const EdgeInsets.all(WsMetrics.space6),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        spacing: WsMetrics.space4,
        children: [
          Text(
            'Writer model',
            style: TextStyle(
              fontSize: WsMetrics.fsUi,
              fontWeight: WsMetrics.fwSemibold,
              color: ws.text,
            ),
          ),
          Text(
            'Writes commit messages left blank, pull request titles and descriptions, and session titles.',
            style: TextStyle(
              fontSize: WsMetrics.fsCompact,
              color: ws.textMuted,
            ),
          ),
          if (_providers case final providers?)
            ModelMenu(
              providers: providers,
              choice: _choice,
              onChanged: (next) => unawaited(_set(next)),
            )
          else
            Text(
              'Reading the models…',
              style: TextStyle(
                fontSize: WsMetrics.fsSmall,
                color: ws.textFaint,
              ),
            ),
          if (_problem case final problem?)
            WarningNotice(problem, danger: true),
        ],
      ),
    );
  }
}
