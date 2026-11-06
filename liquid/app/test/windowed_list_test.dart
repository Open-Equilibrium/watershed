import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/workspace/windowed_list.dart';

void main() {
  test('a reread asks again only for the windows last used', () async {
    final asked = <int>[];
    final windows = WindowedList<int>(
      (start, limit) async {
        asked.add(start);
        return (100000, [for (var at = start; at < start + limit; at++) at]);
      },
      (error) => fail('$error'),
      kept: 5,
    );
    addTearDown(windows.dispose);
    for (var index = 0; index < 2000; index += 100) {
      await windows.load(index);
    }
    asked.clear();
    await windows.reread();
    expect(asked.length, lessThanOrEqualTo(6), reason: '$asked');
    expect(asked, contains(1900), reason: 'the window last used');
    expect(windows.at(1950), 1950);
    expect(windows.at(50), isNull, reason: 'read again once shown');
    expect(await windows.load(50), 50);
  });

  test('waiting for a window while a reread runs returns its item', () async {
    final held = <Completer<void>>[];
    final windows = WindowedList<int>((start, limit) async {
      final release = Completer<void>();
      held.add(release);
      await release.future;
      return (1000, [for (var at = start; at < start + limit; at++) at]);
    }, (error) => fail('$error'));
    addTearDown(windows.dispose);
    final first = windows.load(0);
    held.removeAt(0).complete();
    expect(await first, 0);
    final reread = windows.reread();
    final last = windows.load(950);
    // The window's own read answers while the reread still runs.
    while (held.isNotEmpty) {
      held.removeLast().complete();
      await Future<void>.delayed(Duration.zero);
    }
    await reread;
    while (held.isNotEmpty) {
      held.removeAt(0).complete();
      await Future<void>.delayed(Duration.zero);
    }
    expect(await last.timeout(const Duration(seconds: 1)), 950);
  });

  test('a window read across a whole reread is read again', () async {
    final held = <Completer<void>>[];
    final windows = WindowedList<int>((start, limit) async {
      final release = Completer<void>();
      held.add(release);
      await release.future;
      return (1000, [for (var at = start; at < start + limit; at++) at]);
    }, (error) => fail('$error'));
    addTearDown(windows.dispose);
    final first = windows.load(0);
    held.removeAt(0).complete();
    expect(await first, 0);
    final last = windows.load(950);
    final window = held.removeAt(0);
    // A whole reread runs and ends before the window's read answers.
    final reread = windows.reread();
    await Future<void>.delayed(Duration.zero);
    held.removeAt(0).complete();
    await reread;
    window.complete();
    var answered = false;
    unawaited(last.then((_) => answered = true));
    for (var turn = 0; turn < 10 && !answered; turn++) {
      await Future<void>.delayed(Duration.zero);
      while (held.isNotEmpty) {
        held.removeAt(0).complete();
      }
    }
    expect(await last.timeout(const Duration(seconds: 1)), 950);
  });

  test(
    'a reread asked while one runs returns once the pass it asked for read',
    () async {
      final held = <Completer<void>>[];
      var total = 10;
      final windows = WindowedList<int>((start, limit) async {
        final answer = total;
        final release = Completer<void>();
        held.add(release);
        await release.future;
        return (answer, [for (var at = 0; at < answer; at++) at]);
      }, (error) => fail('$error'));
      addTearDown(windows.dispose);
      final first = windows.reread();
      await Future<void>.delayed(Duration.zero);
      total = 11;
      var second = false;
      final asked = windows.reread().then((_) => second = true);
      held.removeAt(0).complete();
      await first.timeout(const Duration(seconds: 1), onTimeout: () => false);
      await Future<void>.delayed(Duration.zero);
      expect(second, isFalse, reason: 'its own pass has not read yet');
      while (held.isNotEmpty) {
        held.removeAt(0).complete();
        await Future<void>.delayed(Duration.zero);
      }
      await asked;
      expect(windows.count, 11);
    },
  );
}
