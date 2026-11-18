import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter/services.dart';

class PdfPages {
  PdfPages(this.sizes, this._renderPage, this._closeDocument);

  static Future<PdfPages> open(Uint8List bytes) => opener(bytes);

  static Future<PdfPages> Function(Uint8List bytes) opener =
      _openThroughChannel;

  final List<ui.Size> sizes;

  final Future<ui.Image> Function(int index, int width) _renderPage;
  final Future<void> Function() _closeDocument;

  Future<ui.Image> render(int index, int width) => _renderPage(index, width);

  Future<void> close() => _closeDocument();
}

class PdfUnreadable implements Exception {
  const PdfUnreadable();

  @override
  String toString() => 'PdfUnreadable';
}

const _pdfChannel = MethodChannel('liquid/pdf');

Future<Map<String, Object?>> _ask(
  String method,
  Map<String, Object?> arguments,
) async =>
    await _pdfChannel.invokeMapMethod<String, Object?>(method, arguments) ??
    const {};

Future<PdfPages> _openThroughChannel(Uint8List bytes) async {
  final Map<String, Object?> opened;
  try {
    opened = await _ask('open', {'bytes': bytes});
  } on PlatformException catch (error) {
    if (error.code == 'unreadable') throw const PdfUnreadable();
    rethrow;
  } on MissingPluginException {
    throw const PdfUnreadable();
  }
  final id = opened['id'] as int;
  return PdfPages(
    [
      for (final size in opened['sizes'] as List)
        ui.Size((size[0] as num).toDouble(), (size[1] as num).toDouble()),
    ],
    (index, width) => _renderThroughChannel(id, index, width),
    () async => await _pdfChannel.invokeMethod<void>('close', {'id': id}),
  );
}

Future<ui.Image> _renderThroughChannel(int id, int index, int width) async {
  final answer = await _ask('render', {
    'id': id,
    'page': index,
    'width': width,
  });
  final decoded = Completer<ui.Image>();
  ui.decodeImageFromPixels(
    answer['pixels'] as Uint8List,
    answer['width'] as int,
    answer['height'] as int,
    ui.PixelFormat.rgba8888,
    decoded.complete,
  );
  return decoded.future;
}
