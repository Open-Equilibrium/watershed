import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../controls.dart';
import '../rust/api/workspace.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'calculation_block.dart';
import 'chart_block.dart';
import 'code_block.dart';
import 'math_block.dart';
import 'explorer.dart';
import 'media_block.dart';
import 'arrange_bar.dart';
import 'arrange_layout.dart' show arrangedWidth;
import 'arranged_page.dart';
import 'page_views.dart';
import 'page_editor.dart';
import 'paste.dart';
import 'stored_name.dart';
import 'table_block.dart';
import 'text_block.dart';
import 'whiteboard_block.dart';
import 'page_icon.dart';
import 'workspace_state.dart';

class WorkspacePage extends StatefulWidget {
  const WorkspacePage({
    super.key,
    required this.state,
    required this.open,
    this.version = false,
  });

  final bool version;

  final WorkspaceState state;
  final OpenPage open;

  @override
  State<WorkspacePage> createState() => _WorkspacePageState();
}

class _WorkspacePageState extends State<WorkspacePage> implements EditorSource {
  late final _title = StoredName(
    _page?.title ?? '',
    rename: (title) => widget.state.rename(widget.open.page, title),
    restore: () async {
      if (!mounted) return null;
      try {
        await widget.state.readPages(widget.open.area);
      } on Object catch (_) {}
      return mounted ? _page?.title ?? '' : null;
    },
    quitting: () => widget.state.quitting,
    replaced: () {
      if (mounted) setState(() {});
    },
  );
  late final _titleFocus = FocusNode(
    debugLabel: 'Page title',
    onKeyEvent: (_, event) => _titleKey(event),
  );
  late final _views = PageViews(widget.state, widget.open.page)..start();

  final _first = TextEditingController();
  late final _firstFocus = FocusNode(
    debugLabel: 'First line',
    onKeyEvent: (_, event) {
      final copied = widget.state.copiedView;
      if (event is! KeyUpEvent &&
          isPaste(event) &&
          copied != null &&
          _views.ready.value &&
          !_views.editor.isSource(this)) {
        widget.state.track(_pasteFirst(copied));
        return KeyEventResult.handled;
      }
      return event is KeyUpEvent ||
              !_first.value.composing.isCollapsed ||
              !_plain ||
              !_views.editor.isSource(this)
          ? KeyEventResult.ignored
          : _views.editor.queue(this, event);
    },
  );

  Future<void> _pasteFirst(CopiedView copied) async {
    final Pasted pasted;
    try {
      pasted = await readClipboard();
    } on Object catch (error) {
      widget.state.report('Liquid could not read the clipboard: $error');
      return;
    }
    if (!mounted) return;
    if (copied.text == pasted.text && copied.change == pasted.change) {
      if (await _views.pasteCopy(copied, null) || !mounted) return;
      widget.state.copiedView = null;
    }
    final text = pasted.text;
    final selection = _first.selection;
    if (text == null || text.isEmpty || !selection.isValid) return;
    writeText(_first, _first.value.replaced(selection, text));
  }

  static bool get _plain {
    final keys = HardwareKeyboard.instance;
    return !keys.isShiftPressed &&
        !keys.isMetaPressed &&
        !keys.isControlPressed &&
        !keys.isAltPressed;
  }

  final _firstLineKey = GlobalKey(debugLabel: 'first-line');

  bool _handing = false;

  ValueListenable<List<BridgePage>> get _tree =>
      widget.state.pages(widget.open.area);

  BridgePage? get _page =>
      _tree.value.where((page) => page.id == widget.open.page).firstOrNull;

  @override
  void initState() {
    super.initState();
    _tree.addListener(_titleChanged);
    _first.addListener(_typedFirst);
    _views.ready.addListener(_readied);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) widget.state.currentPage.value = _views;
    });
    widget.state.finishing.add(_typedFirst);
    widget.state.finishing.add(_title.typed);
    if (widget.open.fresh) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) _titleFocus.requestFocus();
      });
    }
  }

  @override
  void dispose() {
    final editor = _views.editor;
    if (!editor.isSource(this) && !editor.busy && _first.text.isNotEmpty) {
      editor.first(this);
    }
    editor.left(this);
    if (widget.state.currentPage.value == _views) {
      widget.state.currentPage.value = null;
    }
    widget.state.finishing.remove(_typedFirst);
    widget.state.finishing.remove(_title.typed);
    _tree.removeListener(_titleChanged);
    _title.dispose();
    _titleFocus.dispose();
    _views.dispose();
    _first.dispose();
    _firstFocus.dispose();
    super.dispose();
  }

  void _titleChanged() {
    if (_titleFocus.hasFocus) return;
    final title = _page?.title ?? '';
    if (_title.controller.text != title) _title.show(title);
  }

  void _readied() {
    if (mounted) setState(() {});
  }

  KeyEventResult _titleKey(KeyEvent event) {
    if (event is KeyUpEvent ||
        !_title.controller.value.composing.isCollapsed ||
        !_plain) {
      return KeyEventResult.ignored;
    }
    if (event.logicalKey != LogicalKeyboardKey.enter &&
        event.logicalKey != LogicalKeyboardKey.arrowDown) {
      return KeyEventResult.ignored;
    }
    if (_views.textFrom(0, 1) case final first?) {
      _views.caret.value = Caret(first.id, 0);
    } else if (_views.count == 0) {
      _firstFocus.requestFocus();
    }
    return KeyEventResult.handled;
  }

  void _typedFirst() {
    final editor = _views.editor;
    if (editor.isSource(this)) {
      editor.typed(this);
      return;
    }
    if (_first.text.isEmpty ||
        !_first.value.composing.isCollapsed && !widget.state.quitting) {
      return;
    }
    if (editor.busy) return;
    setState(() => _handing = true);
    editor.first(this);
  }

  @override
  String? get block => null;

  @override
  TextEditingValue get typed => _first.value;

  @override
  bool get focused => _firstFocus.hasFocus;

  @override
  void collapse() {
    final selection = _first.selection;
    if (selection.isValid && !selection.isCollapsed) {
      _first.selection = TextSelection.collapsed(offset: selection.end);
    }
  }

  @override
  void settle(String? text) {
    if (!mounted) return;
    setState(() => _handing = false);
    _first.clear();
  }

  Widget _withFullBoard(Widget page) => ValueListenableBuilder(
    valueListenable: _views.fullBoard,
    builder: (context, board, _) => Stack(
      children: [
        Offstage(offstage: board != null, child: page),
        if (board != null)
          Positioned.fill(
            child: DrawingFullScreen(
              key: ValueKey('full ${board.id}'),
              views: _views,
              view: board,
            ),
          ),
      ],
    ),
  );

  Widget _column(Widget child, {double maxWidth = readingWidth}) => Center(
    child: ConstrainedBox(
      constraints: BoxConstraints(maxWidth: maxWidth),
      child: child,
    ),
  );

  Widget _withUploads(
    BridgeView view,
    int index,
    Widget block, {
    bool wide = false,
  }) => (wide ? (Widget child) => child : _column)(
    ValueListenableBuilder(
      valueListenable: _views.uploads,
      builder: (context, uploads, child) => Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final upload in uploads)
            if (index == 0 && upload.after == null)
              UploadRow(upload, key: ObjectKey(upload)),
          child!,
          for (final upload in uploads)
            if (upload.after == view.id)
              UploadRow(upload, key: ObjectKey(upload)),
        ],
      ),
      child: KeyedSubtree(
        key: const ValueKey('block'),
        child: MediaDropZone(
          views: _views,
          view: view,
          index: index,
          child: block,
        ),
      ),
    ),
  );

  Widget _block(BridgeView view, int index, {required bool wide}) {
    if (view.kind == BridgeBlockKind.text) {
      return KeyedSubtree(
        key: ValueKey(view.id),
        child: _withUploads(
          view,
          index,
          TextBlock(
            key: _views.keyFor(view.id),
            views: _views,
            view: view,
            index: index,
          ),
        ),
      );
    }
    return KeyedSubtree(
      key: ValueKey(view.id),
      child: _withUploads(
        view,
        index,
        Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            if (view.boundary && index > 0) const SectionDivider(),
            switch (view.kind) {
              BridgeBlockKind.database => TableBlock(views: _views, view: view),
              BridgeBlockKind.code => CodeBlock(
                views: _views,
                view: view,
                index: index,
              ),
              BridgeBlockKind.media => MediaBlock(views: _views, view: view),
              BridgeBlockKind.whiteboard => WhiteboardBlock(
                views: _views,
                view: view,
              ),
              BridgeBlockKind.calculation => CalculationBlock(
                views: _views,
                view: view,
              ),
              BridgeBlockKind.chart => ChartBlock(views: _views, view: view),
              BridgeBlockKind.math => MathBlock(views: _views, view: view),
              _ => const _Placeholder(),
            },
          ],
        ),
        wide: wide,
      ),
    );
  }

  var _titleHovered = false;

  Widget _titled(BuildContext context, Widget title) => MouseRegion(
    onEnter: (_) => setState(() => _titleHovered = true),
    onExit: (_) => setState(() => _titleHovered = false),
    child: ListenableBuilder(
      listenable: widget.state.pages(widget.open.area),
      builder: (context, _) {
        final icon = widget.state.iconOf(widget.open.page);
        return Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            if (widget.version)
              if (icon.isNotEmpty)
                Padding(
                  padding: const EdgeInsets.only(right: WsMetrics.space2),
                  child: PageIcon(
                    icon,
                    size: WsMetrics.fsDisplay,
                    color: context.ws.text,
                  ),
                )
              else
                const SizedBox.shrink()
            else
              Padding(
                padding: const EdgeInsets.only(right: WsMetrics.space2),
                child: PageIconButton(
                  state: widget.state,
                  page: widget.open.page,
                  icon: icon,
                  shown: _titleHovered || _title.controller.text.isEmpty,
                ),
              ),
            Expanded(child: title),
          ],
        );
      },
    ),
  );

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final firstLine = KeyedSubtree(
      key: _firstLineKey,
      child: TextField(
        key: const Key('first-line'),
        controller: _first,
        focusNode: _firstFocus,
        selectAllOnFocus: false,
        readOnly: !_views.ready.value,
        style: TextStyle(
          fontSize: WsMetrics.fsProse,
          height: WsMetrics.lhProse / WsMetrics.fsProse,
          color: ws.text,
        ),
        decoration: InputDecoration.collapsed(
          hintText: 'Type to write.',
          hintStyle: TextStyle(color: ws.textFaint),
        ),
      ),
    );
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (!widget.version)
          ValueListenableBuilder(
            valueListenable: _views.arranging,
            builder: (context, arranging, _) => arranging
                ? ArrangeBar(views: _views, title: _title.controller.text)
                : _Breadcrumb(
                    state: widget.state,
                    open: widget.open,
                    onArrange: () => _views.arranging.value = true,
                  ),
          ),
        ValueListenableBuilder(
          valueListenable: widget.state.problem,
          builder: (context, problem, _) => problem == null
              ? const SizedBox.shrink()
              : Padding(
                  padding: const EdgeInsets.symmetric(
                    horizontal: WsMetrics.space7,
                  ),
                  child: WarningNotice(problem, danger: true),
                ),
        ),
        Expanded(
          child: _withFullBoard(
            ListenableBuilder(
              listenable: Listenable.merge([
                _views,
                _views.editor.lingering,
                _views.ready,
                _views.arranging,
              ]),
              builder: (context, _) => CustomScrollView(
                slivers: [
                  SliverPadding(
                    padding: const EdgeInsets.fromLTRB(
                      WsMetrics.space7,
                      28,
                      WsMetrics.space7,
                      WsMetrics.space4,
                    ),
                    sliver: SliverToBoxAdapter(
                      child: _column(
                        maxWidth: _views.arranged || _views.arranging.value
                            ? arrangedWidth
                            : readingWidth,
                        _titled(
                          context,
                          TextField(
                            key: const Key('page-title'),
                            controller: _title.take(),
                            focusNode: _titleFocus,
                            maxLines: null,
                            keyboardType: TextInputType.text,
                            inputFormatters: [
                              FilteringTextInputFormatter.deny(
                                RegExp(r'[\n\r]'),
                                replacementString: ' ',
                              ),
                            ],
                            style: TextStyle(
                              fontSize: WsMetrics.fsDisplay,
                              height: WsMetrics.lhDisplay / WsMetrics.fsDisplay,
                              fontWeight: WsMetrics.fwBold,
                              color: ws.text,
                            ),
                            decoration: InputDecoration.collapsed(
                              hintText: untitled,
                              hintStyle: TextStyle(color: ws.textFaint),
                            ),
                          ),
                        ),
                      ),
                    ),
                  ),
                  SliverPadding(
                    padding: const EdgeInsets.fromLTRB(
                      WsMetrics.space7,
                      0,
                      WsMetrics.space7,
                      WsMetrics.space8,
                    ),
                    sliver: SliverMainAxisGroup(
                      slivers: [
                        if (_views.count == 0 || _handing)
                          SliverToBoxAdapter(
                            key: const ValueKey('first line'),
                            child: Offstage(
                              offstage: _views.count != 0,
                              child: _column(firstLine),
                            ),
                          ),
                        if (_views.editor.lingering.value
                            case (final view, final index)
                            when _views.indexOf(view.id) == null)
                          SliverToBoxAdapter(
                            key: const ValueKey('lingering'),
                            child: Offstage(
                              child: _column(
                                TextBlock(
                                  key: _views.keyFor(view.id),
                                  views: _views,
                                  view: view,
                                  index: index,
                                ),
                              ),
                            ),
                          ),
                        if (_views.arranged || _views.arranging.value)
                          ArrangedPage(
                            key: const ValueKey('arranged'),
                            views: _views,
                            block: _block,
                          )
                        else
                          SliverList.builder(
                            key: const ValueKey('Blocks'),
                            itemCount: _views.count ?? 0,
                            findChildIndexCallback: (key) =>
                                _views.indexOf((key as ValueKey<String>).value),
                            itemBuilder: (context, index) {
                              final view = _views.at(index);
                              if (view == null) {
                                return const SizedBox(
                                  height: WsMetrics.lhProse,
                                );
                              }
                              return _block(view, index, wide: false);
                            },
                          ),
                      ],
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ],
    );
  }
}

class _Placeholder extends StatelessWidget {
  const _Placeholder();

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      height: 40,
      margin: const EdgeInsets.symmetric(vertical: WsMetrics.space1),
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space4),
      decoration: BoxDecoration(
        color: ws.subtle,
        border: Border.all(color: ws.border),
        borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
      ),
      child: Row(
        children: [
          Icon(LucideIcons.puzzle, size: 16, color: ws.textFaint),
          const SizedBox(width: 10),
          Flexible(
            child: Text(
              'This Block needs a newer version of Liquid.',
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(color: ws.textMuted),
            ),
          ),
        ],
      ),
    );
  }
}

class _Breadcrumb extends StatelessWidget {
  const _Breadcrumb({
    required this.state,
    required this.open,
    required this.onArrange,
  });

  final WorkspaceState state;
  final OpenPage open;
  final VoidCallback onArrange;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      key: const Key('page-header'),
      height: WsMetrics.rowHeader,
      padding: const EdgeInsets.only(
        left: WsMetrics.space7,
        right: WsMetrics.space4,
      ),
      alignment: Alignment.centerLeft,
      child: ListenableBuilder(
        listenable: Listenable.merge([state.areas, state.pages(open.area)]),
        builder: (context, _) {
          final area = state.areas.value
              .where((area) => area.id == open.area)
              .firstOrNull;
          final title = state
              .pages(open.area)
              .value
              .where((page) => page.id == open.page)
              .firstOrNull
              ?.title;
          return Row(
            children: [
              Text(area?.name ?? '', style: TextStyle(color: ws.textMuted)),
              Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: WsMetrics.space2,
                ),
                child: Text('›', style: TextStyle(color: ws.textMuted)),
              ),
              if (state.iconOf(open.page) case final icon
                  when icon.isNotEmpty) ...[
                PageIcon(icon, size: 14),
                const SizedBox(width: WsMetrics.space2),
              ],
              Flexible(
                child: Text(
                  title == null || title.isEmpty ? untitled : title,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    color: ws.text,
                    fontWeight: WsMetrics.fwMedium,
                  ),
                ),
              ),
              const Spacer(),
              IconButton(
                tooltip: 'Page history',
                visualDensity: VisualDensity.compact,
                icon: Icon(LucideIcons.clock, size: 14, color: ws.textMuted),
                onPressed: () => state.pageHistory.value = open.page,
              ),
              InkWell(
                key: const Key('arrange'),
                borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                onTap: onArrange,
                child: Padding(
                  padding: const EdgeInsets.symmetric(
                    horizontal: WsMetrics.space2,
                    vertical: WsMetrics.space1,
                  ),
                  child: Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Icon(
                        LucideIcons.layoutGrid,
                        size: 14,
                        color: ws.textMuted,
                      ),
                      const SizedBox(width: WsMetrics.space2),
                      Text(
                        'Arrange',
                        style: TextStyle(
                          fontSize: WsMetrics.fsCompact,
                          color: ws.textMuted,
                        ),
                      ),
                    ],
                  ),
                ),
              ),
            ],
          );
        },
      ),
    );
  }
}
