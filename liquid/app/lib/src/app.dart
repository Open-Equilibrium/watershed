import 'package:flutter/material.dart';

import 'theme/theme.dart';

/// The Liquid app. It holds no native state, so widget tests can build it without the Rust
/// library.
class LiquidApp extends StatelessWidget {
  const LiquidApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Liquid',
      debugShowCheckedModeBanner: false,
      theme: liquidTheme(Brightness.light),
      darkTheme: liquidTheme(Brightness.dark),
      home: const Scaffold(body: SizedBox.expand()),
    );
  }
}
