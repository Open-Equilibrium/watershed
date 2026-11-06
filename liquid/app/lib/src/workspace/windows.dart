import 'dart:async';

import 'package:flutter/foundation.dart';

/// Items read at once: a screenful and more, far below the bridge's window.
const _chunk = 100;

/// A long list read in windows as it is shown, never whole (client architecture rules 3
/// and 5). Each reread asks again for the count and the windows kept: all read so far, or
/// the `kept` last used, whose others are read again once shown.
class Windows<T> extends ChangeNotifier {
  Windows(this._query, this._report, {this.kept});

  /// How many windows it keeps at most, for a list no walk reads far through.
  final int? kept;

  /// Reads the window `start..start + limit` and the count.
  final Future<(int, List<T>)> Function(int start, int limit) _query;
  final void Function(Object error) _report;

  /// How many items there are; null until first read.
  int? get count => _count;
  int? _count;

  final _chunks = <int, List<T>>{};
  final _loading = <int, Future<void>>{};

  /// Windows asked for while a reread ran, which it reads too.
  final _wanted = <int>{};
  int _generation = 0;
  bool _disposed = false;
  bool _reading = false;
  bool _again = false;

  /// The item at `index`, or null outside the list or while its window is read.
  T? at(int index) {
    if (index < 0 || index >= (_count ?? index + 1)) return null;
    final chunk = index ~/ _chunk;
    // Read again as it is shown, then kept as the last used.
    final items = _chunks.remove(chunk);
    if (items == null) {
      unawaited(_load(chunk));
      return null;
    }
    _chunks[chunk] = items;
    final at = index % _chunk;
    return at < items.length ? items[at] : null;
  }

  /// The position of the first read item that passes `test`, or null.
  int? indexWhere(bool Function(T item) test) {
    for (final MapEntry(key: chunk, value: items) in _chunks.entries) {
      final at = items.indexWhere(test);
      if (at >= 0) return chunk * _chunk + at;
    }
    return null;
  }

  /// The item at `index`, read first if its window was not.
  Future<T?> load(int index) async {
    if (index < 0) return null;
    final chunk = index ~/ _chunk;
    if (!_chunks.containsKey(chunk)) await _load(chunk);
    return at(index);
  }

  /// Reads one window; one the list changed under is read again.
  Future<void> _load(int chunk) =>
      _loading[chunk] ??= _read(chunk).whenComplete(() {
        _loading.remove(chunk);
      });

  Future<void> _read(int chunk) async {
    final generation = _generation;
    try {
      final (count, items) = await _query(chunk * _chunk, _chunk);
      if (_disposed) return;
      // A reread running now reads this window too, so no order mixes with another.
      if (_reading) {
        _wanted.add(chunk);
        _again = true;
        return;
      }
      if (generation != _generation) {
        // Read again once this read has left the list of running ones.
        scheduleMicrotask(() => unawaited(_load(chunk)));
        return;
      }
      _count = count;
      _chunks[chunk] = items;
      _trim();
      notifyListeners();
    } on Object catch (error) {
      if (!_disposed) _report(error);
    }
  }

  /// Rereads the count and every window read so far, one reread at a time.
  Future<void> reread() async {
    if (_reading) {
      _again = true;
      return;
    }
    _reading = true;
    try {
      do {
        _again = false;
        final generation = ++_generation;
        final read = <int, List<T>>{};
        var count = 0;
        for (final chunk in {0, ..._chunks.keys, ..._wanted}) {
          final (all, items) = await _query(chunk * _chunk, _chunk);
          count = all;
          read[chunk] = items;
        }
        if (_disposed || generation != _generation) return;
        // Windows read across a change may disagree; read them all again first.
        if (_again) continue;
        _count = count;
        // In the order last used, the first window, read for the count, oldest.
        final used = [..._chunks.keys, ..._wanted];
        _chunks.clear();
        for (final chunk in {0, ...used}) {
          _chunks[chunk] = read[chunk]!;
        }
        _wanted.clear();
        _trim();
        notifyListeners();
      } while (_again);
    } on Object catch (error) {
      if (!_disposed) _report(error);
    } finally {
      _reading = false;
    }
  }

  /// Drops the windows used longest ago beyond the kept ones.
  void _trim() {
    while (_chunks.length > (kept ?? _chunks.length)) {
      _chunks.remove(_chunks.keys.first);
    }
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}
