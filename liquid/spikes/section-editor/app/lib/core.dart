import 'dart:convert';
import 'dart:ffi';
import 'dart:io';

import 'package:ffi/ffi.dart';

typedef _Handle = Pointer<Void>;

final class _Lib {
  _Lib(DynamicLibrary lib)
    : create = lib.lookupFunction<_Handle Function(), _Handle Function()>('sec_new'),
      free = lib.lookupFunction<Void Function(_Handle), void Function(_Handle)>('sec_free'),
      fork = lib.lookupFunction<_Handle Function(_Handle), _Handle Function(_Handle)>('sec_fork'),
      merge = lib.lookupFunction<Void Function(_Handle, _Handle), void Function(_Handle, _Handle)>('sec_merge'),
      apply = lib
          .lookupFunction<
            Pointer<Utf8> Function(_Handle, Pointer<Utf8>),
            Pointer<Utf8> Function(_Handle, Pointer<Utf8>)
          >('sec_apply'),
      spans = lib.lookupFunction<Pointer<Utf8> Function(_Handle), Pointer<Utf8> Function(_Handle)>('sec_spans'),
      strFree = lib.lookupFunction<Void Function(Pointer<Utf8>), void Function(Pointer<Utf8>)>('sec_str_free');

  final _Handle Function() create;
  final void Function(_Handle) free;
  final _Handle Function(_Handle) fork;
  final void Function(_Handle, _Handle) merge;
  final Pointer<Utf8> Function(_Handle, Pointer<Utf8>) apply;
  final Pointer<Utf8> Function(_Handle) spans;
  final void Function(Pointer<Utf8>) strFree;

  String take(Pointer<Utf8> s) {
    final out = s.toDartString();
    strFree(s);
    return out;
  }
}

String _libraryPath() {
  final env = Platform.environment['SECTION_SPIKE_LIB'];
  if (env != null) return env;
  const defined = String.fromEnvironment('SECTION_SPIKE_LIB');
  if (defined.isNotEmpty) return defined;
  final here = File(Platform.resolvedExecutable).parent.path;
  final bundled = '$here/../Frameworks/libsection_spike.dylib';
  if (File(bundled).existsSync()) return bundled;
  return '${Directory.current.path}/../core/target/release/libsection_spike.dylib';
}

final _lib = _Lib(DynamicLibrary.open(_libraryPath()));

/// One Section: an Automerge text whose block markers start its Paragraphs.
class SectionCore {
  SectionCore() : _handle = _lib.create();
  SectionCore._(this._handle);

  final _Handle _handle;

  SectionCore fork() => SectionCore._(_lib.fork(_handle));

  void merge(SectionCore other) => _lib.merge(_handle, other._handle);

  List<Object?> apply(List<Map<String, Object?>> ops) {
    if (ops.isEmpty) return const [];
    final input = jsonEncode(ops).toNativeUtf8();
    try {
      final result = jsonDecode(_lib.take(_lib.apply(_handle, input)));
      if (result is Map) throw StateError('core refused: $result');
      return result as List<Object?>;
    } finally {
      malloc.free(input);
    }
  }

  List<Map<String, Object?>> spans() =>
      (jsonDecode(_lib.take(_lib.spans(_handle))) as List).cast<Map<String, Object?>>();

  void dispose() => _lib.free(_handle);
}
