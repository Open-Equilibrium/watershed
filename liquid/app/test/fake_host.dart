import 'dart:async';
import 'dart:io';

import 'package:liquid/src/shortcuts/keymap.dart';
import 'package:liquid/src/shortcuts/shortcut_store.dart';

class FakeHost implements LiquidKeyHost {
  FakeHost({this.available = true});

  @override
  final bool available;
  final calls = <bool>[];
  Object? failure;

  Completer<void>? gate;

  @override
  Future<void> apply(bool on) async {
    calls.add(on);
    await gate?.future;
    if (failure case final failure?) throw failure;
  }
}

class MemoryStore extends ShortcutStore {
  MemoryStore([this.saved]) : super(File('unused'));

  ShortcutSettings? saved;

  @override
  Future<ShortcutSettings?> load() async => saved;

  @override
  Future<void> save(ShortcutSettings settings) async => saved = settings;
}
