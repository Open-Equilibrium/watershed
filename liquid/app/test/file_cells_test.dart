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

/// The Table's file field (approved Whiteboard Block mockups, section 7, ADR-0305).
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

  const fields = [
    BridgeField(
      id: 'name',
      name: 'Name',
      kind: BridgeFieldKind.text,
      options: [],
    ),
    BridgeField(
      id: 'files',
      name: 'Files',
      kind: BridgeFieldKind.file,
      options: [],
    ),
  ];

  /// A file the fake Workspace keeps as a finished attachment.
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
  );

  /// Shows a Table of bookings whose first row holds `files`.
  Future<void> bookings(
    WidgetTester tester,
    Future<List<BridgeMedia>> Function() files,
  ) async {
    late List<BridgeMedia> held;
    await kit.show(tester, const [
      TablePart([], fields: fields),
    ], before: () {});
    held = await tester.runAsync(files) ?? const [];
    final block = kit.fake.rows.keys.single;
    kit.fake.putRows(kit.page, block, [
      row('r1', 'Hotel', held),
      row('r2', 'Train to Porto', const []),
    ]);
    await tester.pumpAndSettle();
  }

  Finder fileCell(String row) =>
      kit.inTable(find.byKey(ValueKey('cell $row files')));

  /// The row the last write went to and the names of the files it put there.
  String lastWrite() {
    final set = kit.applied(BridgeMutationKind.setCell).last;
    return '${set.row}: ${[for (final file in set.cell!.files) file.name].join(', ')}';
  }

  testWidgets('a file cell shows its files as chips, and how many more', (
    tester,
  ) async {
    await bookings(
      tester,
      () async => [
        await stored('Hotel booking.eml', 48000),
        for (var at = 0; at < 6; at++) await stored('Invoice $at.pdf', 10),
      ],
    );
    expect(
      find.descendant(
        of: fileCell('r1'),
        matching: find.byKey(const ValueKey('file chip Hotel booking.eml')),
      ),
      findsOneWidget,
    );
    expect(
      find.descendant(of: fileCell('r1'), matching: find.textContaining('+')),
      findsOneWidget,
    );
  });

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
    await tester.tap(fileCell('r1'));
    await tester.pumpAndSettle();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(find.text('2 files'), findsOneWidget);
    expect(find.text('Hotel booking.eml · 48 KB'), findsOneWidget);

    saveTo = '/Users/me/Desktop/booking.eml';
    await tester.tap(find.byKey(const ValueKey('file save Hotel booking.eml')));
    await tester.pumpAndSettle();
    expect(kit.fake.saved['/Users/me/Desktop/booking.eml'], hasLength(48000));

    kit.fake.files['/Users/me/CP ticket.pdf'] = Uint8List(5);
    picked.add(
      const MediaSource.file('CP ticket.pdf', '/Users/me/CP ticket.pdf', 5),
    );
    await tester.tap(find.byKey(const Key('add files')));
    await tester.pumpAndSettle();
    expect(
      lastWrite(),
      'r1: Hotel booking.eml, Invoice.pdf, CP ticket.pdf',
      reason: 'after the files it holds',
    );

    await tester.tap(fileCell('r1'));
    await tester.pumpAndSettle();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    await tester.sendKeyEvent(LogicalKeyboardKey.backspace);
    await tester.pumpAndSettle();
    expect(lastWrite(), 'r1: Invoice.pdf, CP ticket.pdf');
  });

  testWidgets('a cell\'s long list scrolls and keeps Add files… in reach', (
    tester,
  ) async {
    await bookings(
      tester,
      () async => [
        for (var at = 0; at < 60; at++) await stored('Receipt $at.pdf', 10),
      ],
    );
    await tester.tap(fileCell('r1'));
    await tester.pumpAndSettle();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
    expect(find.byKey(const Key('add files')).hitTestable(), findsOneWidget);
  });

  testWidgets('files dropped on a file cell go into it, not onto the Page', (
    tester,
  ) async {
    await bookings(tester, () async => const []);
    const path = '/Users/me/CP ticket.pdf';
    kit.fake.files[path] = Uint8List(3);
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

  testWidgets('⌘V pastes copied files into the selected file cell', (
    tester,
  ) async {
    await bookings(tester, () async => const []);
    const path = '/Users/me/tram.png';
    kit.fake.files[path] = Uint8List(4);
    readClipboard = () async => const Pasted(files: [path]);
    await tester.tap(fileCell('r2'));
    await tester.pumpAndSettle();
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyV);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
    expect(lastWrite(), 'r2: tram.png');
  });

  testWidgets('files added while the row moved keep what it got meanwhile', (
    tester,
  ) async {
    await bookings(tester, () async => [await stored('Hotel.eml', 1)]);
    const path = '/Users/me/CP ticket.pdf';
    kit.fake.files[path] = Uint8List(5);
    picked.add(const MediaSource.file('CP ticket.pdf', path, 5));
    kit.fake.uploadGate = Completer<void>();
    await tester.tap(fileCell('r1'));
    await tester.pumpAndSettle();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('add files')));
    await tester.pump();
    // Meanwhile the row moves down and gets another file elsewhere.
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
      for (final name in ['a.pdf', 'b.pdf']) {
        kit.fake.files['/Users/me/$name'] = Uint8List(3);
      }
      readClipboard = () async => const Pasted(files: ['/Users/me/a.pdf']);
      await tester.tap(fileCell('r2'));
      await tester.pumpAndSettle();
      await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.keyV);
      await tester.sendKeyRepeatEvent(LogicalKeyboardKey.keyV);
      await tester.sendKeyRepeatEvent(LogicalKeyboardKey.keyV);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.keyV);
      // A second paste right away, before the first is read back.
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
    for (final name in ['A.pdf', 'B.pdf']) {
      kit.fake.files['/Users/me/$name'] = Uint8List(3);
    }
    Future<void> paste(String name) async {
      readClipboard = () async => Pasted(files: ['/Users/me/$name']);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyV);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
      await tester.pumpAndSettle();
    }

    await tester.tap(fileCell('r2'));
    await tester.pumpAndSettle();
    await paste('A.pdf');
    expect(lastWrite(), 'r2: A.pdf');
    await tester.sendKeyEvent(LogicalKeyboardKey.backspace);
    await tester.pumpAndSettle();
    await paste('B.pdf');
    expect(lastWrite(), 'r2: B.pdf');
  });

  testWidgets(
    'a queued add goes to the row it was made in, wherever it moved',
    (tester) async {
      await bookings(tester, () async => const []);
      for (final name in ['A.pdf', 'B.pdf']) {
        kit.fake.files['/Users/me/$name'] = Uint8List(3);
      }
      kit.fake.uploadGate = Completer<void>();
      Future<void> paste(String name) async {
        readClipboard = () async => Pasted(files: ['/Users/me/$name']);
        await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
        await tester.sendKeyEvent(LogicalKeyboardKey.keyV);
        await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
        await tester.pump();
      }

      await tester.tap(fileCell('r1'));
      await tester.pumpAndSettle();
      await paste('A.pdf');
      await paste('B.pdf');
      // While A goes in, a row lands above: r1 is now second.
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
    kit.fake.files['/Users/me/a.pdf'] = Uint8List(1);
    picked.addAll(
      List.filled(100, const MediaSource.file('a.pdf', '/Users/me/a.pdf', 1)),
    );
    await tester.tap(fileCell('r1'));
    await tester.pumpAndSettle();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('add files')));
    await tester.pumpAndSettle();
    expect(kit.applied(BridgeMutationKind.setCell), isEmpty);
    expect(kit.workspace.problem.value, 'A cell holds at most 100 files.');
  });
}
