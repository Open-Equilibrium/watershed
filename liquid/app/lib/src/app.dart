import 'package:flutter/material.dart';

/// The Liquid app. It holds no native state, so widget tests can build it without the Rust
/// library.
class LiquidApp extends StatelessWidget {
  const LiquidApp({super.key});

  @override
  Widget build(BuildContext context) {
    return const MaterialApp(
      title: 'Liquid',
      debugShowCheckedModeBanner: false,
      home: Scaffold(body: SizedBox.expand()),
    );
  }
}
