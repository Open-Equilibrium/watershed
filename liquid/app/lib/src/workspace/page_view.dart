import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../controls.dart';
import '../rust/api/workspace.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'explorer.dart';
import 'workspace_state.dart';

/// An open Page: its breadcrumb, then its title in the reading column, as the approved Pages
/// mockups show it. A just-created Page takes the keys in its title.
class WorkspacePage extends StatefulWidget {
  const WorkspacePage({super.key, required this.state, required this.open});

  final WorkspaceState state;
  final OpenPage open;

  @override
  State<WorkspacePage> createState() => _WorkspacePageState();
}

class _WorkspacePageState extends State<WorkspacePage> {
  final _title = TextEditingController();
  final _titleFocus = FocusNode(debugLabel: 'Page title');

  ValueListenable<List<BridgePage>> get _tree =>
      widget.state.pages(widget.open.area);

  BridgePage? get _page =>
      _tree.value.where((page) => page.id == widget.open.page).firstOrNull;

  @override
  void initState() {
    super.initState();
    _title.text = _page?.title ?? '';
    _tree.addListener(_titleChanged);
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
          child: SingleChildScrollView(
            padding: const EdgeInsets.fromLTRB(
              WsMetrics.space7,
              28,
              WsMetrics.space7,
              WsMetrics.space7,
            ),
            child: Center(
              child: ConstrainedBox(
                constraints: const BoxConstraints(maxWidth: readingWidth),
                child: TextField(
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
