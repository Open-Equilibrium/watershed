part of 'fake_workspace.dart';

extension _FakeBlocks on FakeWorkspace {
  BridgeApplied? _blocksMutation(BridgeMutation mutation) {
    switch (mutation.kind) {
      case BridgeMutationKind.addMediaBlock:
        final file = attachments[mutation.attachment]!;
        final view = _newView(BridgeBlockKind.media);
        media[view.block] = BridgeMedia(
          name: mutation.text!,
          mediaType: mutation.mediaType ?? '',
          attachment: mutation.attachment!,
          size: file.length,
        );
        return _placeView(mutation, view);
      case BridgeMutationKind.addWhiteboardBlock:
        final view = _newView(BridgeBlockKind.whiteboard);
        whiteboards[view.block] = const BridgeDrawing(
          style: BridgeDrawingStyle.handDrawn,
          elements: [],
        );
        return _placeView(mutation, view);
      case BridgeMutationKind.addCalculationBlock:
        final view = _newView(BridgeBlockKind.calculation);
        calculations[view.block] = [
          line(
            _id(),
            const BridgeLineSetup(
              label: '',
              formula: '',
              style: BridgeLineStyle.plain,
            ),
          ),
        ];
        return _placeView(mutation, view);
      case BridgeMutationKind.addChartBlock:
        final view = _newView(BridgeBlockKind.chart);
        charts[view.block] = const BridgeChartSetup(
          kind: BridgeChartKind.bar,
          value: BridgeChartValue.count,
        );
        return _placeView(mutation, view);
      case BridgeMutationKind.setChart:
        if (!charts.containsKey(mutation.block)) throw 'not found';
        charts[mutation.block!] = mutation.chart!;
        _changed(_blockResource(mutation));
        return _done();
      case BridgeMutationKind.addLine:
        final lines = calculations[mutation.block] ?? (throw 'not found');
        final at = mutation.after == null
            ? 0
            : lines.indexWhere((line) => line.id == mutation.after) + 1;
        final id = _id();
        lines.insert(at, line(id, mutation.lineSetup!));
        _changed(_blockResource(mutation));
        return _done(line: id);
      case BridgeMutationKind.setLine:
        final lines = calculations[mutation.block] ?? (throw 'not found');
        final at = lines.indexWhere((line) => line.id == mutation.line);
        if (at < 0) throw 'not found';
        final formula = mutation.lineSetup!.formula;
        if (formula.trimRight().endsWith('+')) {
          throw "the formula can't be read from character ${formula.length + 1}";
        }
        lines[at] = line(mutation.line!, mutation.lineSetup!);
        _changed(_blockResource(mutation));
        return _done();
      case BridgeMutationKind.removeLine:
        final lines = calculations[mutation.block] ?? (throw 'not found');
        final before = lines.length;
        lines.removeWhere((line) => line.id == mutation.line);
        if (lines.length == before) throw 'not found';
        _changed(_blockResource(mutation));
        return _done();
      case BridgeMutationKind.setDrawingStyle:
        final board = whiteboards[mutation.block] ?? (throw 'not found');
        if (board.style == mutation.drawingStyle) {
          throw 'the write changes nothing';
        }
        whiteboards[mutation.block!] = BridgeDrawing(
          style: mutation.drawingStyle!,
          elements: board.elements,
        );
        _changed(_blockResource(mutation));
        return _done();
      case BridgeMutationKind.editDrawing:
        final board =
            whiteboards[mutation.block] ??
            (media.containsKey(mutation.block)
                ? const BridgeDrawing(
                    style: BridgeDrawingStyle.handDrawn,
                    elements: [],
                  )
                : throw 'not found');
        final edit = mutation.drawing!;
        final elements = [...board.elements];
        for (final removed in edit.remove) {
          final at = elements.indexWhere((element) => element.id == removed);
          if (at < 0) throw 'not found';
          elements.removeAt(at);
        }
        for (final change in edit.change) {
          final at = elements.indexWhere((element) => element.id == change.id);
          if (at < 0) throw 'not found';
          elements[at] = changed(elements[at], change);
        }
        final ids = [for (final _ in edit.add) _id()];
        for (final (index, element) in edit.add.indexed) {
          elements.add(withId(element, ids[index]));
        }
        whiteboards[mutation.block!] = BridgeDrawing(
          style: board.style,
          elements: elements,
        );
        _changed(_blockResource(mutation));
        return _done(elements: ids);
      default:
        return null;
    }
  }

  List<BridgeElement> _withinBudget(List<BridgeElement> elements) {
    final pages = {for (final element in elements) element.docPage}.toList()
      ..sort();
    var budget = readBudget;
    final kept = <int>{};
    for (final page in pages) {
      final count = elements.where((element) => element.docPage == page).length;
      if (count > budget) break;
      budget -= count;
      kept.add(page);
    }
    return [
      for (final element in elements)
        if (kept.contains(element.docPage)) element,
    ];
  }

  String _attachment(Uint8List bytes) {
    final name = Object.hashAll(bytes).toUnsigned(32).toRadixString(16);
    attachments[name.padLeft(64, '0')] = bytes;
    return name.padLeft(64, '0');
  }
}

BridgeElement withId(BridgeElement element, String id) => BridgeElement(
  id: id,
  kind: element.kind,
  x: element.x,
  y: element.y,
  w: element.w,
  h: element.h,
  rotation: element.rotation,
  color: element.color,
  width: element.width,
  docPage: element.docPage,
  pen: element.pen,
  points: element.points,
  shape: element.shape,
  seed: element.seed,
  arrowhead: element.arrowhead,
  corners: element.corners,
  text: element.text,
);

BridgeElement changed(BridgeElement element, BridgeElementChange change) =>
    BridgeElement(
      id: element.id,
      kind: element.kind,
      x: change.x,
      y: change.y,
      w: change.w,
      h: change.h,
      rotation: change.rotation,
      color: change.color,
      width: change.width,
      docPage: element.docPage,
      pen: element.pen,
      points: element.points,
      shape: element.shape,
      seed: element.seed,
      arrowhead: change.arrowhead ?? element.arrowhead,
      corners: element.corners,
      text: change.text ?? element.text,
    );
