import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../controls.dart';
import '../rust/api/workspace.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'explorer.dart';
import 'page_views.dart';
import 'page_editor.dart';
import 'text_block.dart';
import 'workspace_state.dart';

/// An open Page: its breadcrumb, then its title and its Text Blocks in the reading column, as
/// the approved Pages mockups show it. A just-created Page takes the keys in its title; a
/// Page without Blocks offers a line to type in.
class WorkspacePage extends StatefulWidget {
  const WorkspacePage({super.key, required this.state, required this.open});

  final WorkspaceState state;
  final OpenPage open;

  @override
  State<WorkspacePage> createState() => _WorkspacePageState();
}

class _WorkspacePageState extends State<WorkspacePage> implements EditorSource {
  final _title = TextEditingController();
  late final _titleFocus = FocusNode(
    debugLabel: 'Page title',
    onKeyEvent: (_, event) => _titleKey(event),
  );
  late final _views = PageViews(widget.state, widget.open.page)..start();

  /// The line a Page without Blocks offers; typing there creates the first Text Block.
  final _first = TextEditingController();
  late final _firstFocus = FocusNode(
    debugLabel: 'First line',
    // A composition takes its own ⏎, and Flutter's field the keys with modifiers.
    onKeyEvent: (_, event) =>
        event is KeyUpEvent ||
            !_first.value.composing.isCollapsed ||
            !_plain ||
            !_views.editor.isSource(this)
        ? KeyEventResult.ignored
        : _views.editor.queue(this, event),
  );

  static bool get _plain {
    final keys = HardwareKeyboard.instance;
    return !keys.isShiftPressed &&
        !keys.isMetaPressed &&
        !keys.isControlPressed &&
        !keys.isAltPressed;
  }

  /// Keeps the first line, its focus and composition included, while it moves out of sight.
  final _firstLineKey = GlobalKey(debugLabel: 'first-line');

  /// While the first Text Block is made from the line, which stays until it took over.
  bool _handing = false;

  ValueListenable<List<BridgePage>> get _tree =>
      widget.state.pages(widget.open.area);

  BridgePage? get _page =>
      _tree.value.where((page) => page.id == widget.open.page).firstOrNull;

  @override
  void initState() {
    super.initState();
    _title.text = _page?.title ?? '';
    _tree.addListener(_titleChanged);
    _first.addListener(_typedFirst);
    _views.ready.addListener(_readied);
    widget.state.finishing.add(_typedFirst);
    // After the frame, so it wins over the window's own first focus.
    if (widget.open.fresh) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) _titleFocus.requestFocus();
      });
    }
  }

  @override
  void dispose() {
    // A composition still open in the first line is written as it stands.
    final editor = _views.editor;
    if (!editor.isSource(this) && !editor.busy && _first.text.isNotEmpty) {
      editor.first(this);
    }
    editor.left(this);
    widget.state.finishing.remove(_typedFirst);
    _tree.removeListener(_titleChanged);
    _title.dispose();
    _titleFocus.dispose();
    _views.dispose();
    _first.dispose();
    _firstFocus.dispose();
    super.dispose();
  }

  /// A title changed elsewhere shows unless the person is typing one (architecture rule 7).
  void _titleChanged() {
    if (_titleFocus.hasFocus) return;
    final title = _page?.title ?? '';
    if (_title.text != title) _title.text = title;
  }

  /// Each change of the title is one rename; a refused one puts the stored title back.
  void _rename(String title) {
    final rename = ++_renames;
    unawaited(
      widget.state
          .rename(widget.open.page, title)
          .then<void>(
            (_) {},
            onError: (Object _) {
              // Only the latest rename's refusal puts the stored title back.
              if (!mounted || rename != _renames) return;
              _title.text = _page?.title ?? '';
            },
          ),
    );
  }

  int _renames = 0;

  void _readied() {
    if (mounted) setState(() {});
  }

  /// ⏎ or ↓ in the title goes to the first Text Block, as in Notion.
  KeyEventResult _titleKey(KeyEvent event) {
    // A composition takes its own ⏎ and arrows, and the field the keys with modifiers.
    if (event is KeyUpEvent || !_title.value.composing.isCollapsed || !_plain) {
      return KeyEventResult.ignored;
    }
    if (event.logicalKey != LogicalKeyboardKey.enter &&
        event.logicalKey != LogicalKeyboardKey.arrowDown) {
      return KeyEventResult.ignored;
    }
    if (_views.at(0) case final first?) {
      _views.caret.value = Caret(first.id, 0);
    } else if (_views.count == 0) {
      _firstFocus.requestFocus();
    }
    return KeyEventResult.handled;
  }

  /// What is typed on an empty Page, once any composition ends, makes its first Text Block
  /// through the Page's input queue (ADR-0240), which also hands over what is typed
  /// meanwhile and applies a Markdown shortcut typed there.
  void _typedFirst() {
    final editor = _views.editor;
    if (editor.isSource(this)) {
      editor.typed(this);
      return;
    }
    // On quitting, a composition still open is written as it stands.
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

  /// The reading column's width, centred.
  Widget _column(Widget child) => Center(
    child: ConstrainedBox(
      constraints: const BoxConstraints(maxWidth: readingWidth),
      child: child,
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
        // Until the Page's earlier queue is done, nothing can be typed.
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
        _Breadcrumb(state: widget.state, open: widget.open),
        ValueListenableBuilder(
          valueListenable: widget.state.problem,
          builder: (context, problem, _) => problem == null
              ? const SizedBox.shrink()
              : Padding(
                  padding: const EdgeInsets.symmetric(
                    horizontal: WsMetrics.space7,
                  ),
                  child: WarningNotice(problem),
                ),
        ),
        Expanded(
          child: ListenableBuilder(
            listenable: Listenable.merge([
              _views,
              _views.editor.lingering,
              _views.ready,
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
                      TextField(
                        key: const Key('page-title'),
                        controller: _title,
                        focusNode: _titleFocus,
                        onChanged: _rename,
                        // The title wraps but stays one line, as in Notion.
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
                SliverPadding(
                  padding: const EdgeInsets.fromLTRB(
                    WsMetrics.space7,
                    0,
                    WsMetrics.space7,
                    WsMetrics.space8,
                  ),
                  // The line stays until its Text Block took over what was typed.
                  // The first line stays, out of sight, until its Text Block took the
                  // caret, so typing meanwhile still reaches it.
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
                      // The Text Block a change started from stays, out of sight, after
                      // its View is gone, until the change is done.
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
                      SliverList.builder(
                        key: const ValueKey('Text Blocks'),
                        itemCount: _views.count ?? 0,
                        // A Text Block keeps its state, focus included, when Views
                        // above it come or go.
                        findChildIndexCallback: (key) =>
                            _views.indexOf((key as ValueKey<String>).value),
                        itemBuilder: (context, index) {
                          final view = _views.at(index);
                          if (view == null) {
                            return const SizedBox(height: WsMetrics.lhProse);
                          }
                          return KeyedSubtree(
                            key: ValueKey(view.id),
                            child: _column(
                              TextBlock(
                                key: _views.keyFor(view.id),
                                views: _views,
                                view: view,
                                index: index,
                              ),
                            ),
                          );
                        },
                      ),
                    ],
                  ),
                ),
              ],
            ),
          ),
        ),
      ],
    );
  }
}

/// The open Page's area and title, as the header shows them.
class _Breadcrumb extends StatelessWidget {
  const _Breadcrumb({required this.state, required this.open});

  final WorkspaceState state;
  final OpenPage open;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      key: const Key('page-header'),
      height: WsMetrics.rowHeader,
      padding: const EdgeInsets.only(left: WsMetrics.space7),
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
            ],
          );
        },
      ),
    );
  }
}
