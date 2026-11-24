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

enum SettingsSection { appearance, shortcuts, writer, agentHosts, roles }

class SettingsPage extends StatefulWidget {
  const SettingsPage({
    super.key,
    required this.shortcuts,
    required this.section,
    required this.onSection,
    required this.onClose,
    required this.saveFailure,
    required this.onDismissSaveFailure,
    this.appearance,
    this.cardSettings,
    this.writer,
    this.agentHosts,
    this.roles,
  });

  final ShortcutController shortcuts;

  final SettingsSection section;
  final ValueChanged<SettingsSection> onSection;
  final VoidCallback onClose;

  final ValueListenable<SaveFailure?> saveFailure;
  final VoidCallback onDismissSaveFailure;

  final AppearanceController? appearance;

  final SessionCardSettingsController? cardSettings;

  final Widget? writer;

  final Widget? agentHosts;

  final Widget? roles;

  @override
  State<SettingsPage> createState() => _SettingsPageState();
}

class _SettingsPageState extends State<SettingsPage> {
  final _focus = FocusNode(debugLabel: 'settings');

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) => _focus.requestFocus());
  }

  @override
  void dispose() {
    _focus.dispose();
    super.dispose();
  }

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
                        ('Appearance', SettingsSection.appearance),
                      ('Shortcuts', SettingsSection.shortcuts),
                      if (widget.writer != null)
                        ('Writer model', SettingsSection.writer),
                      if (widget.agentHosts != null)
                        ('Agent hosts', SettingsSection.agentHosts),
                      if (widget.roles != null) ...[
                        ('Workspace', null),
                        ('Roles', SettingsSection.roles),
                      ],
                    ])
                      if (section == null)
                        Padding(
                          padding: const EdgeInsets.only(
                            top: WsMetrics.space4,
                            bottom: WsMetrics.space2,
                          ),
                          child: Text(
                            label,
                            style: TextStyle(
                              color: ws.textFaint,
                              fontSize: WsMetrics.fsSmall,
                            ),
                          ),
                        )
                      else
                        Semantics(
                          selected: widget.section == section,
                          child: Shortcuts(
                            shortcuts: activateKeys,
                            child: InkWell(
                              onTap: () => widget.onSection(section),
                              borderRadius: BorderRadius.circular(
                                WsMetrics.radiusMd,
                              ),
                              child: Ink(
                                height: WsMetrics.rowNav,
                                padding: const EdgeInsets.symmetric(
                                  horizontal: WsMetrics.space3,
                                ),
                                decoration: BoxDecoration(
                                  color: widget.section == section
                                      ? ws.selected
                                      : null,
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
                      widget.section,
                      widget.appearance,
                      widget.writer,
                      widget.agentHosts,
                    )) {
                      (SettingsSection.appearance, final appearance?, _, _) =>
                        AppearancePane(
                          appearance: appearance,
                          cardSettings: widget.cardSettings,
                        ),
                      (SettingsSection.writer, _, final writer?, _) => writer,
                      (SettingsSection.agentHosts, _, _, final hosts?) => hosts,
                      (SettingsSection.roles, _, _, _)
                          when widget.roles != null =>
                        widget.roles!,
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
  int _generation = 0;
  Future<void> _saves = Future.value();

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
    final generation = ++_generation;
    setState(() {
      _choice = next;
      _problem = null;
    });
    final save = _saves.then((_) => widget.project.setWriterModel(next));
    _saves = save.catchError((_) {});
    try {
      await save;
    } on Object catch (error) {
      if (mounted && generation == _generation) {
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
