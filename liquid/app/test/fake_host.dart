import 'dart:io';

import 'package:liquid/src/shortcuts/keymap.dart';
import 'package:liquid/src/shortcuts/shortcut_store.dart';

/// Records what the app asks of the Liquid key.
class FakeHost implements LiquidKeyHost {
  FakeHost({this.available = true});

  @override
  final bool available;
  final calls = <bool>[];
  Object? failure;

  @override
  Future<void> apply(bool on) async {
    calls.add(on);
    if (failure case final failure?) throw failure;
  }
}

/// Keeps settings in memory, so widget tests need no files.
class MemoryStore extends ShortcutStore {
  MemoryStore([this.saved]) : super(File('unused'));

  ShortcutSettings? saved;

  @override
  Future<ShortcutSettings?> load() async => saved;

  @override
  Future<void> save(ShortcutSettings settings) async => saved = settings;
}
