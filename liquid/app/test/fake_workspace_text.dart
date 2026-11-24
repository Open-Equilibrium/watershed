part of 'fake_workspace.dart';

extension _FakeText on FakeWorkspace {
  BridgeApplied? _textMutation(BridgeMutation mutation) {
    switch (mutation.kind) {
      case BridgeMutationKind.addTextBlock:
        final view = _newView(BridgeBlockKind.text);
        texts[view.block] = mutation.text!;
        styles[view.block] = BridgeTextStyle.paragraph;
        return _placeView(mutation, view);
      case BridgeMutationKind.editText:
        final text = texts[mutation.block]!;
        texts[mutation.block!] = text.replaceRange(
          mutation.at,
          mutation.at + mutation.delete,
          mutation.text!,
        );
        _changed(_blockResource(mutation));
        return _done();
      case BridgeMutationKind.splitTextBlock:
        final page = views[mutation.page]!;
        final at = page.indexWhere((view) => view.id == mutation.view);
        final text = texts[mutation.block]!;
        final BridgeView created;
        if (mutation.at == 0) {
          final shown = page[at];
          page[at] = _withView(shown, boundary: false);
          created = _block(
            mutation.page!,
            at,
            '',
            mutation.style,
            boundary: shown.boundary,
          );
        } else {
          texts[mutation.block!] = text.substring(0, mutation.at);
          _changed(_blockResource(mutation));
          created = _block(
            mutation.page!,
            at + 1,
            text.substring(mutation.at),
            mutation.style,
          );
        }
        return _done(block: created.block, view: created.id);
      case BridgeMutationKind.joinTextBlock:
        final page = views[mutation.page]!;
        final at = page.indexWhere((view) => view.id == mutation.view);
        if (at <= 0 || page[at - 1].kind != BridgeBlockKind.text) {
          throw 'not found';
        }
        if (page[at].boundary) throw 'a Section starts here';
        final above = page[at - 1].block;
        texts[above] = texts[above]! + texts.remove(page[at].block)!;
        page.removeAt(at);
        _changed(
          BridgeResource(
            kind: BridgeResourceKind.block,
            page: mutation.page,
            block: above,
          ),
        );
        _viewsChanged(mutation.page);
        return _done();
      case BridgeMutationKind.turnIntoCode ||
          BridgeMutationKind.turnIntoMath ||
          BridgeMutationKind.turnIntoText:
        final kind = _kindOf(mutation.page!, mutation.block!);
        final to = switch (mutation.kind) {
          BridgeMutationKind.turnIntoCode => BridgeBlockKind.code,
          BridgeMutationKind.turnIntoMath => BridgeBlockKind.math,
          _ => BridgeBlockKind.text,
        };
        final from = to == BridgeBlockKind.text ? kind : BridgeBlockKind.text;
        if (kind != from ||
            (to == BridgeBlockKind.text &&
                kind != BridgeBlockKind.code &&
                kind != BridgeBlockKind.math)) {
          throw 'not found';
        }
        final page = views[mutation.page]!;
        for (final (at, shown) in page.indexed) {
          if (shown.block != mutation.block) continue;
          page[at] = _withView(shown, kind: to);
        }
        languages.remove(mutation.block);
        if (to == BridgeBlockKind.text) {
          styles[mutation.block!] = BridgeTextStyle.paragraph;
        }
        _viewsChanged(mutation.page);
        _changed(_blockResource(mutation));
        return _done();
      case BridgeMutationKind.setCodeLanguage:
        if (_kindOf(mutation.page!, mutation.block!) != BridgeBlockKind.code) {
          throw 'not found';
        }
        if (mutation.text!.isEmpty) {
          languages.remove(mutation.block);
        } else {
          languages[mutation.block!] = mutation.text!;
        }
        _changed(_blockResource(mutation));
        return _done();
      case BridgeMutationKind.setTextStyle:
        styles[mutation.block!] = mutation.style;
        _changed(_blockResource(mutation));
        return _done();
      default:
        return null;
    }
  }

  BridgeView _block(
    String page,
    int at,
    String text,
    BridgeTextStyle style, {
    bool boundary = false,
  }) {
    final view = _newView(BridgeBlockKind.text, boundary: boundary);
    texts[view.block] = text;
    styles[view.block] = style;
    views[page]!.insert(at, view);
    _viewsChanged(page);
    return view;
  }
}
