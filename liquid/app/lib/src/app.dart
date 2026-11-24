import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_localizations/flutter_localizations.dart';

import 'agent/session_card_settings.dart';
import 'agent/shell.dart';
import 'agent_hosts/agent_hosts.dart';
import 'appearance.dart';
import 'diff/diff_settings.dart';
import 'launch.dart';
import 'settings_page.dart';
import 'shortcuts/shortcut_store.dart';
import 'theme/theme.dart';
import 'workspace/workspace_state.dart';
import 'workspace/workspaces.dart';

final _languages = kMaterialSupportedLanguages.intersection(
  kCupertinoSupportedLanguages,
);

class LiquidApp extends StatefulWidget {
  const LiquidApp({
    super.key,
    required this.launch,
    this.shortcuts,
    this.diffSettings,
    this.cardSettings,
    this.appearance,
    this.agentHosts,
    this.workspace,
    this.workspaceProblem,
    this.workspaces,
  });

  final Workspaces? workspaces;

  final ValueListenable<Launch> launch;

  final WorkspaceState? workspace;
  final String? workspaceProblem;
  final ShortcutController? shortcuts;
  final DiffSettingsController? diffSettings;
  final SessionCardSettingsController? cardSettings;

  final AppearanceController? appearance;

  final AgentHostsController? agentHosts;

  @override
  State<LiquidApp> createState() => _LiquidAppState();
}

class _LiquidAppState extends State<LiquidApp> {
  final _settings = ValueNotifier<SettingsSection?>(null);

  @override
  void dispose() {
    _settings.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final appearance = widget.appearance ?? AppearanceController();
    return ValueListenableBuilder(
      valueListenable: appearance,
      builder: (context, chosen, _) => ValueListenableBuilder(
        valueListenable: widget.launch,
        builder: (context, current, _) => MaterialApp(
          title: 'Liquid',
          debugShowCheckedModeBanner: false,
          theme: liquidTheme(Brightness.light),
          darkTheme: liquidTheme(Brightness.dark),
          themeMode: chosen.theme.mode,
          localizationsDelegates: GlobalMaterialLocalizations.delegates,
          supportedLocales: [
            const Locale('en'),
            for (final language in _languages)
              if (language != 'en') Locale(language),
          ],
          localeListResolutionCallback: (locales, _) =>
              locales
                  ?.where((locale) => _languages.contains(locale.languageCode))
                  .firstOrNull ??
              const Locale('en'),
          builder: (context, child) =>
              UiScale(scale: chosen.textSize.scale, child: child!),
          home: ListenableBuilder(
            listenable: widget.workspaces ?? const _Still(),
            builder: (context, _) => widget.workspaces?.switching ?? false
                ? const SizedBox.shrink()
                : AgentShell(
                    key: ValueKey((current, widget.workspaces?.current)),
                    state: current.state,
                    info: current.info,
                    workspace: widget.workspaces?.current ?? widget.workspace,
                    workspaceProblem:
                        widget.workspaces?.problem ?? widget.workspaceProblem,
                    workspaces: widget.workspaces,
                    agentProblem: current.problem,
                    shortcuts: widget.shortcuts,
                    diffSettings: widget.diffSettings,
                    cardSettings: widget.cardSettings,
                    appearance: appearance,
                    agentHosts: widget.agentHosts,
                    settings: _settings,
                  ),
          ),
        ),
      ),
    );
  }
}

class _Still implements Listenable {
  const _Still();

  @override
  void addListener(VoidCallback listener) {}

  @override
  void removeListener(VoidCallback listener) {}
}
