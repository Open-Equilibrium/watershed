part of 'text_block.dart';

extension _TextBlockInsert on _TextBlockState {
  Future<void> _choose(SlashCommand choice) async {
    _closeMenu();
    _clear();
    if (choice.style case final style?) {
      if (style != _style) _restyle(style);
      return;
    }
    if (choice.block == BridgeBlockKind.code) {
      return _turnInto(BridgeBlockKind.code);
    }
    if (choice.block == BridgeBlockKind.math) {
      return _turnInto(BridgeBlockKind.math);
    }
    if (choice.block == BridgeBlockKind.media) return _addMedia(_pickFiles);
    if (choice.block == BridgeBlockKind.whiteboard) {
      return _addBlock(BridgeMutationKind.addWhiteboardBlock, 'Whiteboard');
    }
    if (choice.block == BridgeBlockKind.chart) {
      return _addBlock(BridgeMutationKind.addChartBlock, 'Chart');
    }
    if (choice.block == BridgeBlockKind.calculation) {
      return _addBlock(BridgeMutationKind.addCalculationBlock, 'Calculation');
    }
    ExistingView? existing;
    if (choice.existing) {
      existing = await pickExistingTable(context, _state);
      if (existing == null || !mounted) return;
    }
    final views = widget.views;
    final view = widget.view;
    final last = widget.index == (views.count ?? 0) - 1;
    _adding = true;
    final BridgeApplied added;
    try {
      String? after = view.id;
      if (last) {
        final above = widget.index > 0
            ? await views.viewAt(widget.index - 1)
            : null;
        if (widget.index > 0 && above == null) {
          _state.report(
            'Liquid did not add the Table, as the Page changed; choose Table again.',
          );
          throw StateError('The Page changed.');
        }
        after = above?.id;
      }
      added = await views.write(switch (existing) {
        final existing? => mutation(
          BridgeMutationKind.addTableView,
          page: _page,
          after: after,
          block: existing.table,
          text: existing.title,
          tabs: existing.tabs,
        ),
        null => mutation(
          BridgeMutationKind.addDatabaseBlock,
          page: _page,
          after: after,
          text: '',
          layout: choice.layout,
        ),
      }, const []);
    } on Object {
      if (mounted) {
        _adding = false;
        _typed();
      } else if (_gone case final gone? when gone.isNotEmpty) {
        unawaited(
          views
              .write(textEdit(_page, block, '', gone), [block])
              .then<void>((_) {}, onError: (Object _) {}),
        );
      }
      return;
    }
    _added = added;
    if (existing != null) {
      _adding = false;
      _added = null;
      views.moveCaret(Caret(added.view!, 0));
    } else if (!mounted) {
      _name(views, added, _gone ?? '');
    } else if (!_focus.hasFocus || _state.quitting) {
      _finish(_text.text);
    } else {
      views.naming[added.view!] = _handOver;
      views.moveCaret(Caret(added.view!, 0));
    }
    try {
      if (view.boundary) {
        await _boundary(views, added.view!, true);
        if (last) await _boundary(views, view.id, false);
      }
      if (!last) {
        await views.write(
          mutation(BridgeMutationKind.removeView, page: _page, view: view.id),
          [block],
        );
      }
    } on Object catch (_) {}
  }

  (String, bool) _handOver() {
    final typed = _text.text;
    _release();
    return (typed, false);
  }

  void _finish(String typed) {
    final added = _added!;
    final views = widget.views;
    if (views.caret.value?.view == added.view) views.caret.value = null;
    _name(views, added, typed);
    _release();
  }

  void _name(PageViews views, BridgeApplied added, String typed) {
    views.naming.remove(added.view);
    if (typed.isEmpty) return;
    unawaited(
      views
          .write(
            mutation(
              BridgeMutationKind.renameView,
              page: _page,
              view: added.view,
              text: typed,
            ),
            [added.view!],
          )
          .then<void>((_) {}, onError: (Object _) {}),
    );
  }

  void _release() {
    widget.views.naming.remove(_added?.view);
    _adding = false;
    _added = null;
    if (mounted) _set('', 0);
  }

  Future<List<MediaSource>> _pickFiles() async {
    try {
      return await MediaPlatform.pickFiles();
    } on Object catch (error) {
      _state.report('Liquid could not open the file dialog: $error');
      return const [];
    }
  }

  Future<void> _addMedia(Future<List<MediaSource>> Function() sources) async {
    final views = widget.views;
    final view = widget.view;
    final index = widget.index;
    final empty = _text.text.isEmpty;
    final last = index == (views.count ?? 0) - 1;
    final added = await sources();
    if (added.isEmpty) return;
    String? after = view.id;
    if (empty && last) {
      final above = index > 0 ? await views.viewAt(index - 1) : null;
      if (index > 0 && above == null) {
        _state.report(
          'Liquid did not add the files, as the Page changed; choose Media again.',
        );
        return;
      }
      after = above?.id;
    }
    final first = await addMedia(
      views,
      added,
      after: after,
      replacing: empty && !last ? view : null,
    );
    if (empty && first != null) await _handOverSection(first, last: last);
  }

  Future<void> _handOverSection(String added, {required bool last}) async {
    final view = widget.view;
    if (!view.boundary) return;
    try {
      await _boundary(widget.views, added, true);
      if (last) await _boundary(widget.views, view.id, false);
    } on Object catch (_) {}
  }

  Future<void> _boundary(PageViews views, String at, bool starts) =>
      views.write(
        mutation(
          BridgeMutationKind.setSectionBoundary,
          page: _page,
          view: at,
          boundary: starts,
        ),
        const [],
      );

  Future<void> _addBlock(BridgeMutationKind kind, String name) async {
    final views = widget.views;
    final view = widget.view;
    final index = widget.index;
    final empty = _text.text.isEmpty;
    final last = index == (views.count ?? 0) - 1;
    String? after = view.id;
    if (empty && last) {
      final above = index > 0 ? await views.viewAt(index - 1) : null;
      if (index > 0 && above == null) {
        _state.report(
          'Liquid did not add the $name, as the Page changed; choose $name again.',
        );
        return;
      }
      after = above?.id;
    }
    try {
      final added = await views.write(
        mutation(kind, page: _page, after: after),
        const [],
      );
      if (kind == BridgeMutationKind.addWhiteboardBlock) {
        views.focusBoard.value = added.view;
      }
      final vacant = empty && _text.text.isEmpty;
      if (vacant) await _handOverSection(added.view!, last: last);
      if (vacant && !last) {
        await views.write(
          mutation(BridgeMutationKind.removeView, page: _page, view: view.id),
          [view.block],
        );
      }
    } on Object catch (_) {}
  }
}
