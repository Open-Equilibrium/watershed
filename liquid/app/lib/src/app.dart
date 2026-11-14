import 'package:flutter/material.dart';
import 'package:flutter_localizations/flutter_localizations.dart';

import 'agent/session_card_settings.dart';
import 'agent/shell.dart';
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
    this.workspace,
    this.workspaceProblem,
  });

  final Launch launch;

  /// The Workspace, or why it did not open.
  final WorkspaceState? workspace;
  final String? workspaceProblem;
  final ShortcutController? shortcuts;
  final DiffSettingsController? diffSettings;
  final SessionCardSettingsController? cardSettings;

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
        home: state != null || workspace != null
            ? AgentShell(
                state: state,
                info: launch.info,
                workspace: workspace,
                workspaceProblem: workspaceProblem,
                shortcuts: shortcuts,
                diffSettings: diffSettings,
                cardSettings: cardSettings,
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
