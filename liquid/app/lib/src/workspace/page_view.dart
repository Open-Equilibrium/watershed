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

class _WorkspacePageState extends State<WorkspacePage> {
  final _title = TextEditingController();
  late final _titleFocus = FocusNode(
    debugLabel: 'Page title',
    onKeyEvent: (_, event) => _titleKey(event),
  );
  late final _views = PageViews(widget.state, widget.open.page)..start();

  /// The line a Page without Blocks offers; typing there creates the first Text Block.
  final _first = TextEditingController();
  final _firstFocus = FocusNode(debugLabel: 'First line');
  bool _creating = false;

  ValueListenable<List<BridgePage>> get _tree =>
      widget.state.pages(widget.open.area);

  BridgePage? get _page =>
      _tree.value.where((page) => page.id == widget.open.page).firstOrNull;

  @override
  void initState() {
    super.initState();
    _title.text = _page?.title ?? '';
    _tree.addListener(_titleChanged);
    _first.addListener(() => unawaited(_typedFirst()));
    // After the frame, so it wins over the window's own first focus.
    if (widget.open.fresh) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) _titleFocus.requestFocus();
      });
    }
  }

  @override
  void dispose() {
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
  void _rename(String title) => unawaited(
    widget.state
        .rename(widget.open.page, title)
        .then<void>(
          (_) {},
          onError: (Object _) {
            if (!mounted) return;
            _title.text = _page?.title ?? '';
          },
        ),
  );

  /// ⏎ or ↓ in the title goes to the first Text Block, as in Notion.
  KeyEventResult _titleKey(KeyEvent event) {
    if (event is KeyUpEvent) return KeyEventResult.ignored;
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

  /// The first character typed on an empty Page, once any composition ends, creates its
  /// first Text Block, which then takes everything typed meanwhile and the caret.
  Future<void> _typedFirst() async {
    final text = _first.text;
    if (_creating || text.isEmpty || !_first.value.composing.isCollapsed) {
      return;
    }
    _creating = true;
    try {
      final created = await widget.state.apply(
        mutation(
          BridgeMutationKind.addTextBlock,
          page: widget.open.page,
          text: text,
        ),
      );
      final typed = _first.value;
      if (typed.text != text) {
        await widget.state.apply(
          textEdit(widget.open.page, created.block!, text, typed.text),
        );
      }
      if (!mounted) return;
      _views.caret.value = Caret(
        created.view!,
        typed.selection.isValid
            ? typed.selection.baseOffset
            : typed.text.length,
      );
      _first.clear();
    } on Object {
      // The problem shows above the Page; what was typed stays in the line.
    } finally {
      _creating = false;
    }
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
            listenable: _views,
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
                  sliver: _views.count == 0
                      ? SliverToBoxAdapter(
                          child: _column(
                            TextField(
                              key: const Key('first-line'),
                              controller: _first,
                              focusNode: _firstFocus,
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
                          ),
                        )
                      : SliverList.builder(
                          itemCount: _views.count ?? 0,
                          // A Text Block keeps its state, focus included, when Views above it
                          // come or go.
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
                                  views: _views,
                                  view: view,
                                  index: index,
                                ),
                              ),
                            );
                          },
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
      height: 44,
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
