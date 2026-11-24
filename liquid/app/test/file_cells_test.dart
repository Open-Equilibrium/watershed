import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/media.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/media_block.dart';
import 'package:liquid/src/workspace/paste.dart';

import 'page_kit.dart';

const fields = [
  BridgeField(
    id: 'name',
    name: 'Name',
    kind: BridgeFieldKind.text,
    options: [],
    formula: '',
  ),
  BridgeField(
    id: 'files',
    name: 'Files',
    kind: BridgeFieldKind.file,
    options: [],
    formula: '',
  ),
];

const bookingTable = TablePart([], fields: fields);

void main() {
  final kit = PageKit();
  final picked = <MediaSource>[];
  String? saveTo;

  setUp(() {
    picked.clear();
    saveTo = null;
    MediaPlatform.pickFiles = () async => [...picked];
    MediaPlatform.saveLocation = (_) async => saveTo;
    MediaPlatform.fileSize = (path) async => kit.fake.files[path]!.length;
  });

  void putFile(String path, int size) => kit.fake.files[path] = Uint8List(size);

  void pick(String path, int size) {
    putFile(path, size);
    picked.add(MediaSource.file(path.split('/').last, path, size));
  }

  Future<BridgeMedia> stored(String name, int size) async => BridgeMedia(
    name: name,
    mediaType: mediaTypeOf(name),
    attachment: await kit.fake.addBytes(bytes: Uint8List(size)),
    size: size,
  );

  BridgeRow row(String id, String name, List<BridgeMedia> files) => BridgeRow(
    id: id,
    cells: [
      BridgeFieldCell(
        field: 'name',
        value: cell(BridgeCellKind.text, text: name),
      ),
      if (files.isNotEmpty)
        BridgeFieldCell(
          field: 'files',
          value: BridgeCell(
            kind: BridgeCellKind.files,
            number: 0,
            checked: false,
            files: files,
          ),
        ),
    ],
    computed: const [],
  );

  Future<void> bookings(
    WidgetTester tester,
    Future<List<BridgeMedia>> Function() files,
  ) async {
    await kit.show(tester, [bookingTable], before: () {});
    final held = await tester.runAsync(files) ?? const [];
    kit.fake.putRows(kit.page, kit.fake.rows.keys.single, [
      row('r1', 'Hotel', held),
      row('r2', 'Train to Porto', const []),
    ]);
    await tester.pumpAndSettle();
  }

  Future<void> tap(WidgetTester tester, Finder finder) async {
    await tester.tap(finder);
    await tester.pumpAndSettle();
  }

  Future<void> press(WidgetTester tester, LogicalKeyboardKey key) async {
    await tester.sendKeyEvent(key);
    await tester.pumpAndSettle();
  }

  Future<void> paste(WidgetTester tester, String name) async {
    readClipboard = () async => Pasted(files: ['/Users/me/$name']);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyV);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pump();
  }

  Finder fileCell(String row) =>
      kit.inTable(find.byKey(ValueKey('cell $row files')));

  String lastWrite() {
    final set = kit.applied(BridgeMutationKind.setCell).last;
    return '${set.row}: ${[for (final file in set.cell!.files) file.name].join(', ')}';
  }

  testWidgets('⏎ opens a cell\'s files: Save as…, Remove and Add files…', (
    tester,
  ) async {
    await bookings(
      tester,
      () async => [
        await stored('Hotel booking.eml', 48000),
        await stored('Invoice.pdf', 120000),
      ],
    );
    await tap(tester, fileCell('r1'));
    await press(tester, LogicalKeyboardKey.enter);
    expect(find.text('2 files'), findsOneWidget);
    expect(find.text('Hotel booking.eml · 48 KB'), findsOneWidget);

    saveTo = '/Users/me/Desktop/booking.eml';
    await tap(
      tester,
      find.byKey(const ValueKey('file save Hotel booking.eml')),
    );
    expect(kit.fake.saved['/Users/me/Desktop/booking.eml'], hasLength(48000));

    pick('/Users/me/CP ticket.pdf', 5);
    await tap(tester, find.byKey(const Key('add files')));
    expect(
      lastWrite(),
      'r1: Hotel booking.eml, Invoice.pdf, CP ticket.pdf',
      reason: 'after the files it holds',
    );

    await tap(tester, fileCell('r1'));
    await press(tester, LogicalKeyboardKey.enter);
    await press(tester, LogicalKeyboardKey.backspace);
    expect(lastWrite(), 'r1: Invoice.pdf, CP ticket.pdf');
  });

  testWidgets('files dropped on a file cell go into it, not onto the Page', (
    tester,
  ) async {
    await bookings(tester, () async => const []);
    const path = '/Users/me/CP ticket.pdf';
    putFile(path, 3);
    Future<void> native(String method, Object arguments) async {
      await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
        'desktop_drop',
        const StandardMethodCodec().encodeMethodCall(
          MethodCall(method, arguments),
        ),
        (_) {},
      );
      await tester.pump();
    }

    final at = tester.getCenter(fileCell('r2'));
    await native('entered', [at.dx, at.dy]);
    await native('updated', [at.dx, at.dy]);
    expect(find.byKey(const Key('drop line')), findsNothing);
    await native('performOperation', [path]);
    await tester.pumpAndSettle();

    expect(lastWrite(), 'r2: CP ticket.pdf');
    expect(
      kit.fake.views[kit.page]!.where(
        (view) => view.kind == BridgeBlockKind.media,
      ),
      isEmpty,
    );
  });

  testWidgets('files added while the row moved keep what it got meanwhile', (
    tester,
  ) async {
    await bookings(tester, () async => [await stored('Hotel.eml', 1)]);
    pick('/Users/me/CP ticket.pdf', 5);
    kit.fake.uploadGate = Completer<void>();
    await tap(tester, fileCell('r1'));
    await press(tester, LogicalKeyboardKey.enter);
    await tester.tap(find.byKey(const Key('add files')));
    await tester.pump();
    final extra = await tester.runAsync(() => stored('Extra.pdf', 2));
    final hotel = await tester.runAsync(() => stored('Hotel.eml', 1));
    kit.fake.putRows(kit.page, kit.fake.rows.keys.single, [
      row('r2', 'Train to Porto', const []),
      row('r1', 'Hotel', [hotel!, extra!]),
    ]);
    await tester.pumpAndSettle();
    kit.fake.uploadGate!.complete();
    await tester.pumpAndSettle();
    expect(lastWrite(), 'r1: Hotel.eml, Extra.pdf, CP ticket.pdf');
  });

  testWidgets(
    'a held ⌘V pastes once, and quick adds keep each other\'s files',
    (tester) async {
      await bookings(tester, () async => const []);
      putFile('/Users/me/a.pdf', 3);
      putFile('/Users/me/b.pdf', 3);
      readClipboard = () async => const Pasted(files: ['/Users/me/a.pdf']);
      await tap(tester, fileCell('r2'));
      await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.keyV);
      await tester.sendKeyRepeatEvent(LogicalKeyboardKey.keyV);
      await tester.sendKeyRepeatEvent(LogicalKeyboardKey.keyV);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.keyV);
      readClipboard = () async => const Pasted(files: ['/Users/me/b.pdf']);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyV);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
      await tester.pumpAndSettle();
      expect(lastWrite(), 'r2: a.pdf, b.pdf');
    },
  );

  testWidgets('a file removed after an add stays removed on the next add', (
    tester,
  ) async {
    await bookings(tester, () async => const []);
    putFile('/Users/me/A.pdf', 3);
    putFile('/Users/me/B.pdf', 3);
    await tap(tester, fileCell('r2'));
    await paste(tester, 'A.pdf');
    await tester.pumpAndSettle();
    expect(lastWrite(), 'r2: A.pdf');
    await press(tester, LogicalKeyboardKey.backspace);
    await paste(tester, 'B.pdf');
    await tester.pumpAndSettle();
    expect(lastWrite(), 'r2: B.pdf');
  });

  testWidgets('two removals before the table reloads keep both removed', (
    tester,
  ) async {
    await bookings(
      tester,
      () async => [
        await stored('A.eml', 1),
        await stored('B.eml', 1),
        await stored('C.eml', 1),
      ],
    );
    kit.fake.deferredKinds = {BridgeQueryKind.rows};
    kit.fake.deferred = [];
    await tap(tester, fileCell('r1'));
    await press(tester, LogicalKeyboardKey.enter);
    await press(tester, LogicalKeyboardKey.backspace);
    await press(tester, LogicalKeyboardKey.backspace);
    expect(lastWrite(), 'r1: C.eml');
  });

  testWidgets(
    'a queued add goes to the row it was made in, wherever it moved',
    (tester) async {
      await bookings(tester, () async => const []);
      putFile('/Users/me/A.pdf', 3);
      putFile('/Users/me/B.pdf', 3);
      kit.fake.uploadGate = Completer<void>();
      await tap(tester, fileCell('r1'));
      await paste(tester, 'A.pdf');
      await paste(tester, 'B.pdf');
      final block = kit.fake.rows.keys.single;
      kit.fake.putRows(kit.page, block, [
        row('r0', 'Flight', const []),
        ...kit.fake.rows[block]!,
      ]);
      await tester.pump();
      kit.fake.uploadGate!.complete();
      await tester.pumpAndSettle();
      final rows = {
        for (final set in kit.applied(BridgeMutationKind.setCell)) set.row,
      };
      expect(rows, {'r1'});
      expect(lastWrite(), 'r1: A.pdf, B.pdf');
    },
  );

  testWidgets('a cell takes at most 100 files', (tester) async {
    await bookings(tester, () async => [await stored('Hotel.eml', 1)]);
    putFile('/Users/me/a.pdf', 1);
    picked.addAll(
      List.filled(100, const MediaSource.file('a.pdf', '/Users/me/a.pdf', 1)),
    );
    await tap(tester, fileCell('r1'));
    await press(tester, LogicalKeyboardKey.enter);
    await tap(tester, find.byKey(const Key('add files')));
    expect(kit.applied(BridgeMutationKind.setCell), isEmpty);
    expect(kit.workspace.problem.value, 'A cell holds at most 100 files.');
  });
}
