import 'package:flutter/material.dart';
import 'package:flutter_localizations/flutter_localizations.dart';

import 'agent/shell.dart';
import 'appearance.dart';
import 'diff/diff_settings.dart';
import 'launch.dart';
import 'shortcuts/shortcut_store.dart';
import 'theme/theme.dart';
import 'workspace/workspace_state.dart';

/// The Liquid app. It holds no native state itself, so widget tests can build it without the
/// Rust library.
class LiquidApp extends StatelessWidget {
  const LiquidApp({
    super.key,
    required this.launch,
    this.shortcuts,
    this.diffSettings,
    this.appearance,
    this.workspace,
    this.workspaceProblem,
  });

  final Launch launch;

  /// The Workspace, or why it did not open.
  final WorkspaceState? workspace;
  final String? workspaceProblem;
  final ShortcutController? shortcuts;
  final DiffSettingsController? diffSettings;

  /// Theme and text size; the system theme at the default size, unsaved, without it.
  final AppearanceController? appearance;

  @override
  Widget build(BuildContext context) {
    final state = launch.state;
    final appearance = this.appearance ?? AppearanceController();
    return ValueListenableBuilder(
      valueListenable: appearance,
      builder: (context, chosen, _) => MaterialApp(
        title: 'Liquid',
        debugShowCheckedModeBanner: false,
        theme: liquidTheme(Brightness.light),
        darkTheme: liquidTheme(Brightness.dark),
        themeMode: chosen.theme.mode,
        // Numbers and days read in the person's language (ADR-0246); English comes first,
        // for a language Material does not know.
        localizationsDelegates: GlobalMaterialLocalizations.delegates,
        supportedLocales: [
          const Locale('en'),
          for (final language in kMaterialSupportedLanguages)
            if (language != 'en') Locale(language),
        ],
        builder: (context, child) =>
            UiScale(scale: chosen.textSize.scale, child: child!),
        home: state != null || workspace != null
            ? AgentShell(
                state: state,
                info: launch.info,
                workspace: workspace,
                workspaceProblem: workspaceProblem,
                shortcuts: shortcuts,
                diffSettings: diffSettings,
                appearance: appearance,
              )
            : Scaffold(
                body: Center(
                  child: Padding(
                    padding: const EdgeInsets.all(32),
                    child: Text(
                      [?workspaceProblem, ?launch.problem].join('\n\n'),
                      textAlign: TextAlign.center,
                    ),
                  ),
                ),
              ),
      ),
    );
  }
}
