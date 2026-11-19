import 'dart:async';

import 'package:flutter/material.dart';

import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'anchored_menu.dart';
import 'stored_name.dart';
import 'workspace_state.dart';
import 'workspaces.dart';

const workspaceIcons = [
  '🏡', '💼', '📚', '🧪', '🎨', '🧳', '💰', '⚽', //
  '🎵', '🌱', '🚀', '🛠️', '📷', '🍳', '🐾', '❤️',
];

const _pill = 100.0;
const _cell = 30.0;

const unnamedWorkspace = 'Workspace';

String _shown(String name) => name.isEmpty ? unnamedWorkspace : name;

class WorkspaceIcon extends StatelessWidget {
  const WorkspaceIcon({
    super.key,
    required this.dir,
    required this.name,
    required this.icon,
  });

  final String dir;
  final String name;
  final String icon;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    if (icon.isNotEmpty) {
      return SizedBox(
        width: 22,
        height: 22,
        child: Center(
          child: Text(icon, style: const TextStyle(fontSize: 15, height: 1)),
        ),
      );
    }
    final colors = [
      (ws.accent, ws.onAccent),
      (ws.data1, ws.onData),
      (ws.data2, ws.onData),
      (ws.data3, ws.onData),
    ];
    final (background, foreground) =
        colors[dir.codeUnits.fold(0, (sum, unit) => sum + unit) %
            colors.length];
    return Container(
      width: 22,
      height: 22,
      alignment: Alignment.center,
      decoration: BoxDecoration(
        color: background,
        borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
      ),
      child: Text(
        _shown(name).characters.first.toUpperCase(),
        style: TextStyle(
          color: foreground,
          fontSize: 12,
          fontWeight: WsMetrics.fwSemibold,
        ),
      ),
    );
  }
}

class WorkspaceSwitcher extends StatelessWidget {
  const WorkspaceSwitcher({
    super.key,
    required this.state,
    required this.workspaces,
  });

  final WorkspaceState state;
  final Workspaces? workspaces;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return ListenableBuilder(
      listenable: Listenable.merge([state.named, ?workspaces]),
      builder: (context, _) {
        final workspaces = this.workspaces;
        final named = state.named.value;
        final entries =
            workspaces?.entries ??
            [(dir: 'Workspace', name: named.name, icon: named.icon)];
        final at = workspaces?.at ?? 0;
        final current = entries[at];
        return LayoutBuilder(
          builder: (context, constraints) {
            final others = [
              for (final (index, entry) in entries.indexed)
                if (index != at) (index, entry),
            ];
            final room = ((constraints.maxWidth - _pill - _cell) / _cell)
                .floor()
                .clamp(0, others.length);
            final shown = others.length <= room + 1
                ? others
                : others.take(room).toList();
            final hidden = others.skip(shown.length).toList();
            return SizedBox(
              height: WsMetrics.rowSide,
              child: Row(
                children: [
                  Expanded(
                    child: _Current(
                      state: state,
                      dir: current.dir,
                      named: named,
                      alone: entries.length == 1,
                      workspaces: workspaces,
                    ),
                  ),
                  for (final (index, entry) in shown)
                    Tooltip(
                      message: index < 9
                          ? '${_shown(entry.name)} · ⌘${index + 1}'
                          : _shown(entry.name),
                      child: InkWell(
                        key: Key('workspace $index'),
                        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                        onTap: () => unawaited(workspaces?.switchTo(index)),
                        child: SizedBox(
                          width: _cell,
                          height: 28,
                          child: Center(
                            child: WorkspaceIcon(
                              dir: entry.dir,
                              name: entry.name,
                              icon: entry.icon,
                            ),
                          ),
                        ),
                      ),
                    ),
                  if (hidden.isNotEmpty)
                    _More(hidden: hidden, workspaces: workspaces!)
                  else if (workspaces != null)
                    Tooltip(
                      message: 'New Workspace',
                      child: InkWell(
                        key: const Key('new workspace'),
                        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                        onTap: () => unawaited(workspaces.create()),
                        child: SizedBox(
                          width: _cell,
                          height: 28,
                          child: Icon(
                            LucideIcons.plus,
                            size: 14,
                            color: ws.textMuted,
                          ),
                        ),
                      ),
                    ),
                ],
              ),
            );
          },
        );
      },
    );
  }
}

class _Current extends StatefulWidget {
  const _Current({
    required this.state,
    required this.dir,
    required this.named,
    required this.alone,
    required this.workspaces,
  });

  final bool alone;
  final Workspaces? workspaces;

  final WorkspaceState state;
  final String dir;
  final ({String name, String icon}) named;

  @override
  State<_Current> createState() => _CurrentState();
}

class _CurrentState extends State<_Current> {
  final _menu = OverlayPortalController();
  late final _name = StoredName(
    widget.named.name,
    rename: widget.state.renameWorkspace,
    restore: () async => mounted ? widget.state.named.value.name : null,
    quitting: () => widget.state.quitting,
    replaced: () {
      if (mounted) setState(() {});
    },
  );
  final _focus = FocusNode(debugLabel: 'Workspace name');

  @override
  void initState() {
    super.initState();
    widget.state.finishing.add(_name.typed);
    if (widget.workspaces?.takeFresh() ?? false) {
      WidgetsBinding.instance.addPostFrameCallback((_) => _open());
    }
  }

  @override
  void didUpdateWidget(_Current old) {
    super.didUpdateWidget(old);
    if (widget.named.name != old.named.name && !_focus.hasFocus) {
      _name.show(widget.named.name);
    }
  }

  @override
  void dispose() {
    widget.state.finishing.remove(_name.typed);
    _name.write();
    _name.dispose();
    _focus.dispose();
    super.dispose();
  }

  void _open() {
    if (!mounted) return;
    setState(_menu.show);
    _focus.requestFocus();
    _name.take().selection = TextSelection(
      baseOffset: 0,
      extentOffset: _name.take().text.length,
    );
  }

  void _close() {
    _name.write();
    if (mounted && _menu.isShowing) setState(_menu.hide);
  }

  void _icon(String icon) =>
      unawaited(widget.state.setWorkspaceIcon(icon).catchError((_) {}));

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final named = widget.named;
    return AnchoredMenu(
      controller: _menu,
      height: 220,
      onTapOutside: (_) => _close(),
      menu: (_) => Container(
        key: const Key('workspace menu'),
        width: 216,
        padding: const EdgeInsets.all(WsMetrics.space3),
        decoration: BoxDecoration(
          color: ws.surface,
          borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
          boxShadow: ws.shadow,
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          spacing: WsMetrics.space2,
          children: [
            Container(
              height: 30,
              padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space2),
              alignment: Alignment.centerLeft,
              decoration: BoxDecoration(
                color: ws.canvas,
                borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                border: Border.all(color: ws.accent, width: 1.5),
              ),
              child: TextField(
                key: const Key('workspace name'),
                controller: _name.take(),
                focusNode: _focus,
                onSubmitted: (_) => _close(),
                style: TextStyle(fontSize: WsMetrics.fsUi, color: ws.text),
                decoration: InputDecoration.collapsed(
                  hintText: unnamedWorkspace,
                  hintStyle: TextStyle(color: ws.textFaint),
                ),
              ),
            ),
            Text(
              'Icon',
              style: TextStyle(
                fontSize: WsMetrics.fsSmall,
                color: ws.textFaint,
              ),
            ),
            GridView.count(
              crossAxisCount: 8,
              shrinkWrap: true,
              physics: const NeverScrollableScrollPhysics(),
              children: [
                for (final icon in workspaceIcons)
                  InkWell(
                    borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                    onTap: () => _icon(icon),
                    child: Container(
                      alignment: Alignment.center,
                      decoration: BoxDecoration(
                        color: icon == named.icon ? ws.hover : null,
                        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                      ),
                      child: Text(icon, style: const TextStyle(fontSize: 16)),
                    ),
                  ),
              ],
            ),
            InkWell(
              borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
              onTap: () => _icon(''),
              child: Container(
                height: 28,
                padding: const EdgeInsets.symmetric(
                  horizontal: WsMetrics.space1,
                ),
                child: Row(
                  spacing: WsMetrics.space2,
                  children: [
                    WorkspaceIcon(dir: widget.dir, name: named.name, icon: ''),
                    Flexible(
                      child: Text(
                        'Use the first letter',
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontSize: WsMetrics.fsCompact,
                          color: ws.textMuted,
                        ),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ],
        ),
      ),
      child: InkWell(
        key: const Key('current workspace'),
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        onTap: () => _menu.isShowing ? _close() : _open(),
        child: Container(
          height: 28,
          padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space2),
          decoration: BoxDecoration(
            color: widget.alone ? null : ws.selected,
            borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
          ),
          child: Row(
            spacing: WsMetrics.space2,
            children: [
              WorkspaceIcon(
                dir: widget.dir,
                name: named.name,
                icon: named.icon,
              ),
              Expanded(
                child: Text(
                  _shown(named.name),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    fontSize: WsMetrics.fsSide,
                    fontWeight: WsMetrics.fwMedium,
                    color: ws.text,
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _More extends StatefulWidget {
  const _More({required this.hidden, required this.workspaces});

  final List<(int, WorkspaceEntry)> hidden;
  final Workspaces workspaces;

  @override
  State<_More> createState() => _MoreState();
}

class _MoreState extends State<_More> {
  final _menu = OverlayPortalController();

  void _close() {
    if (mounted && _menu.isShowing) setState(_menu.hide);
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    Widget item(Widget leading, String label, VoidCallback onTap, Key key) =>
        InkWell(
          key: key,
          borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
          onTap: () {
            _close();
            onTap();
          },
          child: Container(
            height: 32,
            padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space2),
            child: Row(
              spacing: WsMetrics.space3,
              children: [
                leading,
                Expanded(
                  child: Text(
                    label,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(fontSize: WsMetrics.fsUi, color: ws.text),
                  ),
                ),
              ],
            ),
          ),
        );
    return AnchoredMenu(
      controller: _menu,
      end: true,
      height: 32.0 * (widget.hidden.length + 1) + 8,
      onTapOutside: (_) => _close(),
      menu: (_) => Container(
        width: 200,
        padding: const EdgeInsets.all(WsMetrics.space1),
        decoration: BoxDecoration(
          color: ws.surface,
          borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
          boxShadow: ws.shadow,
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            for (final (index, entry) in widget.hidden)
              item(
                WorkspaceIcon(
                  dir: entry.dir,
                  name: entry.name,
                  icon: entry.icon,
                ),
                _shown(entry.name),
                () => unawaited(widget.workspaces.switchTo(index)),
                Key('workspace $index'),
              ),
            item(
              Icon(LucideIcons.plus, size: 14, color: ws.textMuted),
              'New Workspace',
              () => unawaited(widget.workspaces.create()),
              const Key('new workspace'),
            ),
          ],
        ),
      ),
      child: InkWell(
        key: const Key('more workspaces'),
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        onTap: () => setState(_menu.toggle),
        child: SizedBox(
          width: _cell,
          height: 28,
          child: Center(
            child: Text(
              '+${widget.hidden.length}',
              style: TextStyle(
                fontSize: WsMetrics.fsSmall,
                color: ws.textMuted,
              ),
            ),
          ),
        ),
      ),
    );
  }
}
