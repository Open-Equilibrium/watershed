import 'package:flutter/foundation.dart';

import '../rust/api/history.dart';

/// What the model saw for the latest responses opened, read when first asked for. Each context
/// lists the branch before it, so only the latest few stay.
class ContextDetails extends ChangeNotifier {
  ContextDetails(this._read);

  final Future<BridgeContextDetail?> Function(String entry) _read;
  final _details = <String, BridgeContextDetail?>{};

  /// Why what the model saw for a response could not be read, by entry.
  final _unread = <String, String>{};
  static const _kept = 4;
  bool _disposed = false;

  /// The context of the response `record`, starting its read the first time; none for other
  /// steps or while it is read.
  BridgeContextDetail? detailFor(BridgeRecord record) {
    if (record.kind != BridgeRecordKind.message) return null;
    final entry = record.entry;
    if (!_details.containsKey(entry)) {
      if (_details.length == _kept) {
        final oldest = _details.keys.first;
        _details.remove(oldest);
        _unread.remove(oldest);
      }
      _details[entry] = null;
      _read(entry).then(
        (detail) => _settle(entry, () {
          if (detail == null) {
            _unread[entry] = 'the session has no such response';
          } else {
            _details[entry] = detail;
          }
        }),
        onError: (Object error) =>
            _settle(entry, () => _unread[entry] = '$error'),
      );
    }
    return _details[entry];
  }

  /// Why the context of `entry` could not be read.
  String? unread(String entry) => _unread[entry];

  /// Applies a finished read, unless its entry was dropped meanwhile.
  void _settle(String entry, VoidCallback apply) {
    if (_disposed || !_details.containsKey(entry)) return;
    apply();
    notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}
