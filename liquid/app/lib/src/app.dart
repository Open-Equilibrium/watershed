import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_localizations/flutter_localizations.dart';

import 'agent/session_card_settings.dart';
import 'agent/shell.dart';
import 'agent_hosts/agent_hosts.dart';
import 'appearance.dart';
import 'diff/diff_settings.dart';
import 'launch.dart';
import 'shortcuts/shortcut_store.dart';
import 'theme/theme.dart';
import 'workspace/workspace_state.dart';

/// The languages whose texts Material and Cupertino, which macOS's text menu shows, both
/// have.
final _languages = kMaterialSupportedLanguages.intersection(
  kCupertinoSupportedLanguages,
);

/// The Liquid app. It holds no native state itself, so widget tests can build it without the
/// Rust library.
class LiquidApp extends StatelessWidget {
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
  });

  /// The agent session Liquid shows; a new one replaces the shown one at once (ADR-0295).
  final ValueListenable<Launch> launch;

  /// The Workspace, or why it did not open.
  final WorkspaceState? workspace;
  final String? workspaceProblem;
  final ShortcutController? shortcuts;
  final DiffSettingsController? diffSettings;
  final SessionCardSettingsController? cardSettings;

  /// Theme and text size; the system theme at the default size, unsaved, without it.
  final AppearanceController? appearance;

  /// The agent hosts Settings lists (ADR-0295).
  final AgentHostsController? agentHosts;

  @override
  Widget build(BuildContext context) {
    final appearance = this.appearance ?? AppearanceController();
    return ValueListenableBuilder(
      valueListenable: appearance,
      builder: (context, chosen, _) => ValueListenableBuilder(
        valueListenable: launch,
        builder: (context, current, _) => MaterialApp(
          title: 'Liquid',
          debugShowCheckedModeBanner: false,
          theme: liquidTheme(Brightness.light),
          darkTheme: liquidTheme(Brightness.dark),
          themeMode: chosen.theme.mode,
          // Numbers and days read as in the person's region, Material's and Cupertino's
          // texts in its language, English for one either does not know (ADR-0246).
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
          home: current.state != null || workspace != null
              ? AgentShell(
                  key: ObjectKey(current),
                  state: current.state,
                  info: current.info,
                  workspace: workspace,
                  workspaceProblem: workspaceProblem,
                  agentProblem: current.problem,
                  shortcuts: shortcuts,
                  diffSettings: diffSettings,
                  cardSettings: cardSettings,
                  appearance: appearance,
                  agentHosts: agentHosts,
                )
              : Scaffold(
                  body: Center(
                    child: Padding(
                      padding: const EdgeInsets.all(32),
                      child: Text(
                        [?workspaceProblem, ?current.problem].join('\n\n'),
                        textAlign: TextAlign.center,
                      ),
                    ),
                  ),
                ),
        ),
      ),
    );
  }
}
