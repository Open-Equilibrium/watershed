import 'dart:ui' as ui;

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/workspace/pdf_pages.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  const channel = MethodChannel('liquid/pdf');
  final messenger =
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
  final calls = <MethodCall>[];
  final pdf = Uint8List.fromList([37, 80, 68, 70]);

  Object? reply(MethodCall call) => switch (call.method) {
    'open' => {
      'id': 7,
      'sizes': [
        [612.0, 792.0],
        [300.0, 200.5],
      ],
    },
    'render' => {
      'width': 2,
      'height': 1,
      'pixels': Uint8List.fromList([255, 0, 0, 255, 0, 0, 255, 255]),
    },
    _ => null,
  };

  void serve(Object? Function(MethodCall call) answer) {
    messenger.setMockMethodCallHandler(channel, (call) async {
      calls.add(call);
      return answer(call);
    });
  }

  tearDown(() {
    messenger.setMockMethodCallHandler(channel, null);
    calls.clear();
  });

  test('open answers the page sizes in points', () async {
    serve(reply);
    final pages = await PdfPages.open(pdf);
    expect(pages.sizes, [const ui.Size(612, 792), const ui.Size(300, 200.5)]);
    expect(calls.single.method, 'open');
    expect(calls.single.arguments, {'bytes': pdf});
  });

  test('render decodes the RGBA answer into an image of its size', () async {
    serve(reply);
    final pages = await PdfPages.open(pdf);
    final image = await pages.render(1, 2);
    expect((image.width, image.height), (2, 1));
    expect(calls.last.method, 'render');
    expect(calls.last.arguments, {'id': 7, 'page': 1, 'width': 2});
    final pixels = await image.toByteData(format: ui.ImageByteFormat.rawRgba);
    expect(pixels!.buffer.asUint8List(), [255, 0, 0, 255, 0, 0, 255, 255]);
  });

  test('close sends the document id', () async {
    serve(reply);
    final pages = await PdfPages.open(pdf);
    await pages.close();
    expect(calls.last.method, 'close');
    expect(calls.last.arguments, {'id': 7});
  });

  test('a document the renderer refuses throws PdfUnreadable', () async {
    serve((call) => throw PlatformException(code: 'unreadable'));
    await expectLater(PdfPages.open(pdf), throwsA(isA<PdfUnreadable>()));
  });

  test('a platform without the renderer throws PdfUnreadable', () async {
    await expectLater(PdfPages.open(pdf), throwsA(isA<PdfUnreadable>()));
  });
}
