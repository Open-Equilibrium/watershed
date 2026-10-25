import 'package:flutter/material.dart';

import 'agent/shell.dart';
import 'launch.dart';
import 'shortcuts/shortcut_store.dart';
import 'theme/theme.dart';

/// The Liquid app. It holds no native state itself, so widget tests can build it without the
/// Rust library.
class LiquidApp extends StatelessWidget {
  const LiquidApp({super.key, required this.launch, this.shortcuts});

  final Launch launch;
  final ShortcutController? shortcuts;

  @override
  Widget build(BuildContext context) {
    final state = launch.state;
    return MaterialApp(
      title: 'Liquid',
      debugShowCheckedModeBanner: false,
      theme: liquidTheme(Brightness.light),
      darkTheme: liquidTheme(Brightness.dark),
      home: state != null
          ? AgentShell(state: state, info: launch.info!, shortcuts: shortcuts)
          : Scaffold(
              body: Center(
                child: Padding(
                  padding: const EdgeInsets.all(32),
                  child: Text(
                    launch.problem ?? '',
                    textAlign: TextAlign.center,
                  ),
                ),
              ),
            ),
    );
  }
}
