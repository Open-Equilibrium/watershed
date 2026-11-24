import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:intl/intl.dart';

import 'explorer.dart' show untitled;
import '../controls.dart';
import '../rust/api/chart.dart';
import '../rust/api/source.dart';
import '../rust/api/workspace.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'source_cards.dart';
import 'source_kind.dart';
import 'source_listing.dart';
import 'source_pdf.dart';
import 'source_table.dart';
import 'stored_name.dart';
import 'table_view_parts.dart' show MoreMenu;
import 'workspace_state.dart';

abstract final class SourceOpener {
  static Future<ProcessResult> Function(String program, List<String> arguments)
  run = Process.run;
}

class SourcePage extends StatefulWidget {
  const SourcePage({super.key, required this.state, required this.open});

  final WorkspaceState state;
  final OpenPage open;

  @override
  State<SourcePage> createState() => _SourcePageState();
}

class _SourcePageState extends State<SourcePage> {
  BridgeFolder? _folder;
  BridgeSourceFile? _file;
  List<(String, String, String)> _users = const [];
  String? _path;
  DateTime? _readAt;
  StreamSubscription<void>? _watch;
  StreamSubscription<String>? _located;
  Timer? _settle;
  int _reads = 0;
  late final _name = StoredName(
    _entry?.name ?? '',
    rename: (name) =>
        _state.workspace.renameSource(sourceId: _source, name: name),
    restore: () async => mounted ? _entry?.name : null,
    quitting: () => _state.quitting,
    replaced: () {
      if (mounted) setState(() {});
    },
  );
  final _nameFocus = FocusNode(debugLabel: 'Source name');

  WorkspaceState get _state => widget.state;
  String get _source => widget.open.source!;
  String get _inside => widget.open.inside;

  BridgeSource? get _entry => _state
      .sources(widget.open.area)
      .value
      .where((source) => source.id == _source)
      .firstOrNull;

  @override
  void initState() {
    super.initState();
    _state.finishing.add(_name.typed);
    _nameFocus.addListener(() {
      if (!_nameFocus.hasFocus) _name.write();
    });
    _state.sources(widget.open.area).addListener(_renamed);
    _rewatch();
    _located = _state.located.listen((source) {
      if (source == _source) _rewatch();
    });
  }

  void _rewatch() {
    unawaited(_watch?.cancel());
    unawaited(_read());
    _watch = _state.watchSource(_source).listen((_) {
      _settle?.cancel();
      _settle = Timer(
        const Duration(milliseconds: 200),
        () => unawaited(_read()),
      );
    });
  }

  @override
  void dispose() {
    _state.finishing.remove(_name.typed);
    _state.sources(widget.open.area).removeListener(_renamed);
    unawaited(_watch?.cancel());
    unawaited(_located?.cancel());
    _settle?.cancel();
    _name.dispose();
    _nameFocus.dispose();
    super.dispose();
  }

  BridgeSourceKind? _kind;

  void _renamed() {
    final entry = _entry;
    if (entry != null && !_nameFocus.hasFocus) _name.show(entry.name);
    if (entry != null && entry.kind != _kind) unawaited(_read());
    if (mounted) setState(() {});
  }

  Future<void> _read() async {
    final read = ++_reads;
    _kind = _entry?.kind;
    final workspace = _state.workspace;
    try {
      final path = await workspace.sourcePath(sourceId: _source);
      BridgeFolder? folder;
      BridgeSourceFile? file;
      if (_entry?.kind == BridgeSourceKind.folder) {
        folder = await workspace.sourceFolder(
          sourceId: _source,
          inside: _inside,
        );
        if (folder.read != BridgeRead.read && _inside.isNotEmpty) folder = null;
      }
      if (folder == null) {
        file = await workspace.sourceFile(sourceId: _source, inside: _inside);
      }
      final users = <(String, String, String)>[];
      if (_inside.isEmpty && file?.shown == BridgeFileView.table) {
        for (final user in await workspace.sourceUsers(sourceId: _source)) {
          users.add((user.page, user.block, await _chartTitle(user)));
        }
      }
      if (!mounted || read != _reads) return;
      setState(() {
        _path = path;
        _folder = folder;
        _file = file;
        _users = users;
        _readAt = DateTime.now();
      });
    } on Object catch (error) {
      _state.readFailed(error);
    }
  }

  Future<String> _chartTitle(BridgeSourceUser user) async {
    final answer = await _state.read(
      query(BridgeQueryKind.views, page: user.page, limit: 1000),
    );
    final title = answer.views
        .where((view) => view.block == user.block)
        .firstOrNull
        ?.title;
    return title == null || title.isEmpty ? 'Chart' : title;
  }

  String? get _absolute => _file?.path.isNotEmpty == true
      ? _file!.path
      : _path == null
      ? null
      : _inside.isEmpty
      ? _path
      : '$_path/$_inside';

  Future<void> _reveal(String path) =>
      _system(switch (Platform.operatingSystem) {
        'macos' => ('open', ['-R', path]),
        'windows' => ('explorer', ['/select,$path']),
        _ => ('xdg-open', [File(path).parent.path]),
      });

  Future<void> _launch(String path) =>
      _system(switch (Platform.operatingSystem) {
        'macos' => ('open', [path]),
        'windows' => ('explorer', [path]),
        _ => ('xdg-open', [path]),
      });

  Future<void> _system((String, List<String>) command) async {
    try {
      final result = await SourceOpener.run(command.$1, command.$2);
      if (result.exitCode != 0) {
        _state.report('Liquid could not open it: ${result.stderr}');
      }
    } on ProcessException catch (error) {
      _state.report('Liquid could not open it: ${error.message}');
    }
  }

  void _open(String inside) => _state.open.value = OpenPage.source(
    widget.open.area,
    _source,
    inside: inside,
  );

  Future<void> _remove() async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (_) => DangerDialog(
        title: 'Remove the Source “${_entry?.name ?? ''}”?',
        action: 'Remove Source',
        children: [
          Text(
            'Liquid stops reading it. The file stays where it is, and History '
            'can bring the Source back.',
            style: DangerDialog.text(context),
          ),
        ],
      ),
    );
    if (confirmed == true) await _state.removeSource(_source);
  }

  void _openPage(String page) {
    final area = _state.areas.value
        .where(
          (area) =>
              _state.pages(area.id).value.any((known) => known.id == page),
        )
        .firstOrNull;
    if (area != null) _state.open.value = OpenPage(area.id, page);
  }

  Future<void> _useInBlock(String page) async {
    final file = _file;
    if (file == null) return;
    final group = file.columns.where((column) => !column.number).firstOrNull;
    final value = file.columns.where((column) => column.number).firstOrNull;
    try {
      final added = await _state.apply(
        mutation(BridgeMutationKind.addChartBlock, page: page),
      );
      await _state.apply(
        mutation(
          BridgeMutationKind.setChart,
          page: page,
          block: added.block,
          chart: BridgeChartSetup(
            kind: BridgeChartKind.bar,
            source: _source,
            group: group?.id,
            value: value == null
                ? BridgeChartValue.count
                : BridgeChartValue.sum,
            field: value?.id,
          ),
        ),
      );
      _openPage(page);
    } on Object catch (error) {
      _state.report('Liquid did not add the Chart: $error');
    }
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final entry = _entry;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _header(context, entry),
        ValueListenableBuilder(
          valueListenable: _state.problem,
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
          child: entry == null
              ? Center(
                  child: Text(
                    'This Source was removed.',
                    style: TextStyle(color: ws.textFaint),
                  ),
                )
              : _body(context, entry),
        ),
      ],
    );
  }

  Widget _header(BuildContext context, BridgeSource? entry) {
    final ws = context.ws;
    final area = _state.areas.value
        .where((area) => area.id == widget.open.area)
        .firstOrNull;
    final parts = _inside.isEmpty ? const <String>[] : _inside.split('/');
    final crumbs = <(String, String?)>[
      (area?.name ?? '', null),
      (entry?.name ?? '', parts.isEmpty ? null : ''),
      for (final (at, part) in parts.indexed)
        (part, at == parts.length - 1 ? null : parts.take(at + 1).join('/')),
    ];
    final muted = TextStyle(color: ws.textMuted);
    return Container(
      key: const Key('source-header'),
      height: WsMetrics.rowHeader,
      padding: const EdgeInsets.only(
        left: WsMetrics.space7,
        right: WsMetrics.space4,
      ),
      child: Row(
        children: [
          Expanded(
            child: Row(
              children: [
                for (final (at, (label, target)) in crumbs.indexed) ...[
                  if (at > 0)
                    Padding(
                      padding: const EdgeInsets.symmetric(
                        horizontal: WsMetrics.space2,
                      ),
                      child: Text('›', style: muted),
                    ),
                  Flexible(
                    child: target == null
                        ? Text(
                            label,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: at == crumbs.length - 1
                                ? TextStyle(
                                    color: ws.text,
                                    fontWeight: WsMetrics.fwMedium,
                                  )
                                : muted,
                          )
                        : InkWell(
                            onTap: () => _open(target),
                            child: Text(label, maxLines: 1, style: muted),
                          ),
                  ),
                ],
              ],
            ),
          ),
          GhostButton(
            label: 'Reload',
            icon: LucideIcons.refreshCw,
            onPressed: () => unawaited(_read()),
          ),
          if (_absolute case final path?)
            GhostButton(
              label: Platform.isMacOS ? 'Show in Finder' : 'Show in folder',
              onPressed: () => unawaited(_reveal(path)),
            ),
          MoreMenu(
            label: 'Source actions',
            items: [
              (
                LucideIcons.file,
                entry?.kind == BridgeSourceKind.folder
                    ? 'Choose folder…'
                    : 'Choose file…',
                () => unawaited(
                  _state.locateSource(
                    _source,
                    folder: entry?.kind == BridgeSourceKind.folder,
                  ),
                ),
              ),
              (LucideIcons.trash, 'Remove Source', () => unawaited(_remove())),
            ],
          ),
        ],
      ),
    );
  }

  String _clock(DateTime at) => DateFormat.Hm('en_US').format(at);

  Widget _body(BuildContext context, BridgeSource entry) {
    final ws = context.ws;
    final folder = _folder;
    final file = _file;
    final reading = folder == null && file == null;
    final missing =
        folder?.read == BridgeRead.missing || file?.read == BridgeRead.missing;
    final caption = TextStyle(
      fontSize: WsMetrics.fsCompact,
      color: ws.textFaint,
    );
    final read = _readAt == null ? '' : ' · read at ${_clock(_readAt!)}';
    final title = _inside.isEmpty ? entry.name : _inside.split('/').last;
    final info = switch ((folder, file)) {
      _
          when _inside.isNotEmpty &&
              file != null &&
              file.read == BridgeRead.read =>
        'In the Source ${entry.name} · ${kindOf(file.name)} · '
            '${fileSize(file.size.toInt())}$read',
      _ when _inside.isNotEmpty => 'In the Source ${entry.name}$read',
      _ when missing =>
        'Source · local ${entry.kind == BridgeSourceKind.folder ? 'folder' : 'file'} '
            '${_path ?? ''}',
      (final folder?, _) =>
        'Source · local folder ${_path ?? ''} · read-only$read'
            ' · ${_counted(folder.entries)}',
      (_, final file?) when file.shown == BridgeFileView.table =>
        'Source · local file ${_path ?? ''} · read-only$read · '
            '${file.rows.length} ${file.rows.length == 1 ? 'row' : 'rows'}',
      _ => 'Source · local file ${_path ?? ''} · read-only$read',
    };
    final children = <Widget>[
      Row(
        children: [
          Icon(
            sourceIcon(
              title,
              folder:
                  folder != null ||
                  (reading && entry.kind == BridgeSourceKind.folder),
            ),
            size: 28,
            color: ws.textFaint,
          ),
          const SizedBox(width: WsMetrics.space3),
          Expanded(
            child: _inside.isEmpty
                ? TextField(
                    key: const Key('source name'),
                    controller: _name.take(),
                    focusNode: _nameFocus,
                    onSubmitted: (_) => _nameFocus.unfocus(),
                    style: TextStyle(
                      fontSize: WsMetrics.fsH1,
                      height: WsMetrics.lhH1 / WsMetrics.fsH1,
                      fontWeight: WsMetrics.fwBold,
                      color: ws.text,
                    ),
                    decoration: InputDecoration.collapsed(
                      hintText: untitled,
                      hintStyle: TextStyle(color: ws.textFaint),
                    ),
                  )
                : Text(
                    title,
                    style: TextStyle(
                      fontSize: WsMetrics.fsH1,
                      height: WsMetrics.lhH1 / WsMetrics.fsH1,
                      fontWeight: WsMetrics.fwBold,
                      color: ws.text,
                    ),
                  ),
          ),
        ],
      ),
      const SizedBox(height: WsMetrics.space2),
      Text(info, key: const Key('source caption'), style: caption),
      const SizedBox(height: WsMetrics.space4),
    ];
    if (reading) {
      return _column(children);
    }
    if (missing) {
      children.add(_missing(context, entry));
      return _column(children);
    }
    if (folder != null) {
      children
        ..add(
          Expanded(
            child: SourceListing(
              entries: folder.entries,
              inside: _inside,
              onOpen: _open,
            ),
          ),
        )
        ..add(const SizedBox(height: WsMetrics.space3))
        ..add(
          Text('Liquid reads this folder and never writes it.', style: caption),
        );
      return _column(children, fill: true);
    }
    final shown = file!;
    void launch() => unawaited(_launch(shown.path));
    switch (shown.read) {
      case BridgeRead.tooLarge:
        children.add(
          SourceCard(
            text: 'Liquid reads at most 50 MiB of a file.',
            path: shown.path,
            onOpen: launch,
          ),
        );
        return _column(children);
      case BridgeRead.unreadable:
        children.add(
          SourceCard(
            text: 'Liquid can\'t read this file as text.',
            path: shown.path,
            onOpen: launch,
          ),
        );
        return _column(children);
      case BridgeRead.missing || BridgeRead.read:
        break;
    }
    switch (shown.shown) {
      case BridgeFileView.table:
        return SourceTable(
          state: _state,
          file: shown,
          head: children,
          users: _users,
          root: _inside.isEmpty,
          onPage: (page) => unawaited(_useInBlock(page)),
          onOpenPage: _openPage,
        );
      case BridgeFileView.text:
        children.add(
          Container(
            key: const Key('source text'),
            padding: const EdgeInsets.all(WsMetrics.space4),
            decoration: BoxDecoration(
              borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
              border: Border.all(color: ws.border),
            ),
            child: SelectableText(shown.text ?? ''),
          ),
        );
      case BridgeFileView.image:
        children.add(
          Image.memory(shown.bytes!, key: const Key('source image')),
        );
      case BridgeFileView.pdf:
        children.add(
          Expanded(
            child: SourcePdf(key: ValueKey(shown.path), bytes: shown.bytes!),
          ),
        );
        return _column(children, fill: true);
      case BridgeFileView.other:
        children.add(
          SourceCard(
            text: 'Liquid shows no preview for this kind of file.',
            path: shown.path,
            onOpen: launch,
          ),
        );
    }
    return _column(children);
  }

  String _counted(List<BridgeFileEntry> entries) {
    final folders = entries.where((entry) => entry.folder).length;
    final files = entries.length - folders;
    return [
      if (folders > 0) '$folders ${folders == 1 ? 'folder' : 'folders'}',
      '$files ${files == 1 ? 'file' : 'files'}',
    ].join(', ');
  }

  Widget _column(List<Widget> children, {bool fill = false}) {
    const padding = EdgeInsets.fromLTRB(
      WsMetrics.space7,
      28,
      WsMetrics.space7,
      WsMetrics.space8,
    );
    final column = Center(
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: WsMetrics.widthReading),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: children,
        ),
      ),
    );
    return fill
        ? Padding(padding: padding, child: column)
        : SingleChildScrollView(padding: padding, child: column);
  }

  Widget _missing(BuildContext context, BridgeSource entry) {
    final ws = context.ws;
    final folder = _inside.isEmpty && entry.kind == BridgeSourceKind.folder;
    return Container(
      key: const Key('source missing'),
      padding: const EdgeInsets.all(WsMetrics.space4),
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
        border: Border.all(color: ws.border),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        spacing: WsMetrics.space3,
        children: [
          Text(
            'Liquid can\'t find this ${folder ? 'folder' : 'file'} on this device.',
            style: const TextStyle(fontWeight: WsMetrics.fwSemibold),
          ),
          Text(
            'It may have been moved, renamed or deleted. Blocks that use it '
            'show nothing until it is back.',
            style: TextStyle(
              fontSize: WsMetrics.fsCompact,
              color: ws.textMuted,
            ),
          ),
          if (_inside.isEmpty)
            Row(
              spacing: WsMetrics.space2,
              children: [
                PrimaryButton(
                  label: folder ? 'Choose folder…' : 'Choose file…',
                  onPressed: () =>
                      unawaited(_state.locateSource(_source, folder: folder)),
                ),
                SecondaryButton(
                  label: 'Remove Source',
                  onPressed: () => unawaited(_remove()),
                ),
              ],
            ),
        ],
      ),
    );
  }
}
