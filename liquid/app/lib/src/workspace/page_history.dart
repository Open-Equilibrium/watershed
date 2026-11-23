import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:intl/intl.dart';

import '../controls.dart';
import '../rust/api/history.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'automations.dart' show DialogHeader;
import 'explorer.dart' show untitled;
import 'history_dialog.dart' show actorName, dayOf;
import 'page_view.dart';
import 'table_view_parts.dart' show MoreMenu;
import 'workspace_state.dart';

class PageHistoryDialog extends StatefulWidget {
  const PageHistoryDialog({
    super.key,
    required this.state,
    required this.page,
    required this.onClose,
  });

  final WorkspaceState state;
  final String page;
  final VoidCallback onClose;

  @override
  State<PageHistoryDialog> createState() => _PageHistoryDialogState();
}

class _PageHistoryDialogState extends State<PageHistoryDialog> {
  List<BridgeVersion> _versions = const [];
  BridgeVersion? _chosen;
  WorkspaceState? _preview;
  int _reads = 0;

  WorkspaceState get _state => widget.state;

  ({String area, String title})? get _page {
    for (final area in _state.areas.value) {
      for (final page in _state.pages(area.id).value) {
        if (page.id == widget.page) return (area: area.id, title: page.title);
      }
    }
    return null;
  }

  String get _title {
    final title = _page?.title ?? '';
    return title.isEmpty ? untitled : title;
  }

  @override
  void initState() {
    super.initState();
    unawaited(_readVersions());
  }

  @override
  void dispose() {
    unawaited(_close(_preview));
    super.dispose();
  }

  Future<void> _close(WorkspaceState? preview) async {
    if (preview == null) return;
    await preview.dispose();
    preview.workspace.dispose();
  }

  Future<void> _readVersions() async {
    try {
      final versions = await _state.workspace.versions(page: widget.page);
      if (!mounted) return;
      setState(() => _versions = versions);
      if (versions.firstOrNull case final newest?) await _choose(newest);
    } on Object catch (error) {
      _state.readFailed(error);
    }
  }

  Future<void> _choose(BridgeVersion version) async {
    final read = ++_reads;
    setState(() => _chosen = version);
    try {
      final workspace = await _state.workspace.preview(
        page: widget.page,
        to: version.after,
      );
      final preview = WorkspaceState(workspace);
      await preview.start(show: false);
      if (!mounted || read != _reads) {
        await _close(preview);
        return;
      }
      final old = _preview;
      setState(() => _preview = preview);
      await _close(old);
    } on Object catch (error) {
      _state.readFailed(error);
    }
  }

  Future<void> _restore(BridgeVersion version) async {
    try {
      await _state.workspace.restore(page: widget.page, to: version.after);
    } on Object catch (error) {
      _state.report('Liquid did not restore this version: $error');
      return;
    }
    await _readVersions();
  }

  Future<void> _purge() async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (_) => DangerDialog(
        title: 'Delete the earlier versions of “$_title”?',
        action: 'Delete earlier versions',
        children: [
          Text(
            'The Page stays as it is now. Its ${_versions.length} '
            '${_versions.length == 1 ? 'version' : 'versions'}, and everything '
            'only they still hold, such as deleted text, leave this device '
            'for good. History keeps its entries without their content, and '
            'none of them can be reverted afterwards. Tables it shows keep '
            'their own versions; delete those in their menu.',
            style: DangerDialog.text(context),
          ),
        ],
      ),
    );
    if (confirmed != true || !mounted) return;
    try {
      await _state.workspace.purge(page: widget.page);
    } on Object catch (error) {
      _state.report('Liquid did not delete the earlier versions: $error');
      return;
    }
    await _readVersions();
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return CallbackShortcuts(
      bindings: {
        const SingleActivator(LogicalKeyboardKey.escape): widget.onClose,
      },
      child: Stack(
        children: [
          Positioned.fill(
            child: GestureDetector(
              onTap: widget.onClose,
              child: ColoredBox(color: Colors.black.withValues(alpha: 0.18)),
            ),
          ),
          Center(
            child: Focus(
              autofocus: true,
              child: Container(
                key: const Key('page history'),
                width: 1080,
                height: 680,
                clipBehavior: Clip.antiAlias,
                decoration: BoxDecoration(
                  color: ws.surface,
                  borderRadius: BorderRadius.circular(WsMetrics.radiusXl),
                  boxShadow: ws.shadow,
                ),
                child: Material(
                  type: MaterialType.transparency,
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      DialogHeader(
                        title: Text.rich(
                          TextSpan(
                            text: 'Page history ',
                            children: [
                              TextSpan(
                                text: '· $_title',
                                style: TextStyle(
                                  fontWeight: WsMetrics.fwRegular,
                                  color: ws.textMuted,
                                ),
                              ),
                            ],
                          ),
                        ),
                        actions: [
                          MoreMenu(
                            label: 'Page history actions',
                            items: [
                              (
                                LucideIcons.trash,
                                'Delete earlier versions…',
                                () => unawaited(_purge()),
                              ),
                            ],
                          ),
                        ],
                        onClose: widget.onClose,
                      ),
                      Expanded(
                        child: Row(
                          crossAxisAlignment: CrossAxisAlignment.stretch,
                          children: [
                            Expanded(child: _shown(context)),
                            VerticalDivider(width: 1, color: ws.border),
                            SizedBox(width: 300, child: _list(context)),
                          ],
                        ),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }

  Widget _shown(BuildContext context) {
    final ws = context.ws;
    final chosen = _chosen;
    final preview = _preview;
    final page = _page;
    final current = chosen == _versions.firstOrNull;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Container(
          height: 44,
          padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space5),
          color: ws.subtle,
          child: Row(
            children: [
              Expanded(
                child: Text(
                  chosen == null
                      ? 'No versions yet'
                      : current
                      ? 'The current version · read-only'
                      : 'Version from ${_when(chosen)} · read-only',
                  style: TextStyle(
                    fontSize: WsMetrics.fsCompact,
                    color: ws.textMuted,
                  ),
                ),
              ),
              PrimaryButton(
                label: 'Restore version',
                onPressed: chosen == null || current
                    ? null
                    : () => unawaited(_restore(chosen)),
              ),
            ],
          ),
        ),
        Expanded(
          child: preview == null || page == null
              ? const SizedBox.shrink()
              : ExcludeFocus(
                  child: WorkspacePage(
                    key: ValueKey(chosen?.after),
                    state: preview,
                    open: OpenPage(page.area, widget.page),
                  ),
                ),
        ),
      ],
    );
  }

  String _when(BridgeVersion version) {
    final at = DateTime.fromMillisecondsSinceEpoch(version.at.toInt());
    return '${dayOf(at, DateTime.now())} ${DateFormat.Hm('en_US').format(at)}';
  }

  Widget _list(BuildContext context) {
    final ws = context.ws;
    return ListView(
      padding: const EdgeInsets.all(WsMetrics.space3),
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(
            WsMetrics.space2,
            0,
            WsMetrics.space2,
            WsMetrics.space2,
          ),
          child: Text(
            'Versions of $_title',
            style: TextStyle(fontSize: WsMetrics.fsSmall, color: ws.textFaint),
          ),
        ),
        for (final version in _versions)
          InkWell(
            borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
            onTap: () => unawaited(_choose(version)),
            child: Container(
              padding: const EdgeInsets.symmetric(
                horizontal: WsMetrics.space3,
                vertical: WsMetrics.space2,
              ),
              decoration: BoxDecoration(
                color: version == _chosen ? ws.selected : null,
                borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
              ),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(_when(version)),
                  Text(
                    version.savePoint
                        ? 'Save point'
                        : actorName(_state, version.actor),
                    style: TextStyle(
                      fontSize: WsMetrics.fsSmall,
                      color: ws.textFaint,
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
