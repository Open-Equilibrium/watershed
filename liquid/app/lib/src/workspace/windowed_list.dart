import 'dart:async';

import 'package:flutter/foundation.dart';

const windowItems = 100;

class WindowedList<T> extends ChangeNotifier {
  WindowedList(this._query, this._report, {this.kept});

  final int? kept;

  final Future<(int, List<T>)> Function(int start, int limit) _query;
  final void Function(Object error) _report;

  int? get count => _count;
  int? _count;

  final _chunks = <int, List<T>>{};
  final _loading = <int, Future<void>>{};

  final _wanted = <int>{};
  int _generation = 0;
  bool _disposed = false;
  bool _reading = false;
  bool _again = false;

  Future<void>? _rereading;

  int _failures = 0;

  T? at(int index) {
    if (index < 0 || index >= (_count ?? index + 1)) return null;
    final chunk = index ~/ windowItems;
    final items = _chunks.remove(chunk);
    if (items == null) {
      if (!_disposed) unawaited(_load(chunk));
      return null;
    }
    _chunks[chunk] = items;
    final at = index % windowItems;
    return at < items.length ? items[at] : null;
  }

  T? peek(int index) {
    if (index < 0) return null;
    final items = _chunks[index ~/ windowItems];
    final at = index % windowItems;
    return items != null && at < items.length ? items[at] : null;
  }

  int? indexWhere(bool Function(T item) test) {
    for (final MapEntry(key: chunk, value: items) in _chunks.entries) {
      final at = items.indexWhere(test);
      if (at >= 0) return chunk * windowItems + at;
    }
    return null;
  }

  Future<T?> load(int index) async {
    if (index < 0) return null;
    final chunk = index ~/ windowItems;
    final failures = _failures;
    for (var tries = 0; tries < 3; tries++) {
      if (_chunks.containsKey(chunk) || _disposed || _failures != failures) {
        break;
      }
      await (_rereading ?? _load(chunk));
    }
    return _chunks.containsKey(chunk) ? at(index) : null;
  }

  Future<void> _load(int chunk) =>
      _loading[chunk] ??= _read(chunk).whenComplete(() {
        _loading.remove(chunk);
      });

  Future<void> _read(int chunk) async {
    final generation = _generation;
    try {
      final (count, items) = await _query(chunk * windowItems, windowItems);
      if (_disposed) return;
      if (_reading) {
        _wanted.add(chunk);
        _again = true;
        return;
      }
      if (generation != _generation) {
        scheduleMicrotask(() => unawaited(_load(chunk)));
        return;
      }
      _count = count;
      _chunks[chunk] = items;
      _trim();
      notifyListeners();
    } on Object catch (error) {
      _failures++;
      if (!_disposed) _report(error);
    }
  }

  Future<void> reread() async {
    if (_reading) {
      _again = true;
      return _rereading;
    }
    _reading = true;
    final done = Completer<void>();
    _rereading = done.future;
    try {
      do {
        _again = false;
        final generation = ++_generation;
        final read = <int, List<T>>{};
        var count = 0;
        for (final chunk in {0, ..._chunks.keys, ..._wanted}) {
          final (all, items) = await _query(chunk * windowItems, windowItems);
          count = all;
          read[chunk] = items;
        }
        if (_disposed || generation != _generation) return;
        if (_again) continue;
        _count = count;
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
      _failures++;
      if (!_disposed) _report(error);
    } finally {
      _reading = false;
      _rereading = null;
      done.complete();
    }
  }

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
