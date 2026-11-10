import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

import '../hook/build.dart';

void main() {
  test('the build log leaves out the environment the Rust builder lists', () {
    final log = StringBuffer();
    buildLog(log)
      ..config({'GH_TOKEN': 'not-for-logs'})
      ..info('Running cargo build');

    expect(log.toString(), contains('Running cargo build'));
    expect(log.toString(), isNot(contains('not-for-logs')));
  });

  test(
    'the hook watches the manifest of every local crate the bridge builds',
    () {
      final app = Directory.current.uri;
      final workspace = File('../../Cargo.toml').readAsStringSync();
      final crates = {
        for (final match in RegExp(
          r'^([\w-]+) = \{ path = "([^"]+)"',
          multiLine: true,
        ).allMatches(workspace))
          match[1]!: match[2]!,
      };
      // Crate directories from the repository root, closed over local path dependencies.
      final built = <String>{};
      final pending = ['liquid/liquid-bridge'];
      while (pending.isNotEmpty) {
        final dir = pending.removeLast();
        if (!built.add(dir)) continue;
        final manifest = File('../../$dir/Cargo.toml').readAsStringSync();
        for (final match in RegExp(
          r'^([\w-]+)\.workspace = true',
          multiLine: true,
        ).allMatches(manifest)) {
          final path = crates[match[1]];
          if (path != null) pending.add(path);
        }
      }

      final watched = {for (final file in bridgeInputs) app.resolve(file)};
      for (final dir in built) {
        expect(watched, contains(app.resolve('../../$dir/Cargo.toml')));
      }
    },
  );
}
