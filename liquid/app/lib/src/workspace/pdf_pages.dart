import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter/services.dart';

/// A PDF the system's renderer opened (PDFKit on macOS, ADR-0307): its pages' sizes in
/// points, and each page drawn on demand at a width in device pixels.
class PdfPages {
  /// The sizes, then the render and close callbacks; tests fake documents with them.
  PdfPages(this.sizes, this._renderPage, this._closeDocument);

  /// Opens `bytes`; throws [PdfUnreadable] for data the renderer cannot read.
  static Future<PdfPages> open(Uint8List bytes) => opener(bytes);

  /// Replaced by tests and by platforms without a renderer.
  static Future<PdfPages> Function(Uint8List bytes) opener =
      _openThroughChannel;

  /// Each page's size in points, as displayed: crop box and rotation applied.
  final List<ui.Size> sizes;

  final Future<ui.Image> Function(int index, int width) _renderPage;
  final Future<void> Function() _closeDocument;

  /// Page `index` drawn `width` pixels wide, as an image; the renderer caps the width.
  Future<ui.Image> render(int index, int width) => _renderPage(index, width);

  /// Drops the document in the renderer.
  Future<void> close() => _closeDocument();
}

/// Data the renderer cannot read, or a locked PDF, or no renderer on this platform.
class PdfUnreadable implements Exception {
  const PdfUnreadable();

  @override
  String toString() => 'PdfUnreadable';
}

const _pdfChannel = MethodChannel('liquid/pdf');

/// The map a method answers with; no answer is an empty map.
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

/// Page `index` as the renderer answers it: RGBA, 8 bits, row-major, without padding.
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
