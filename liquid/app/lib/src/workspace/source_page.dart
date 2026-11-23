import 'dart:async';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:intl/intl.dart';

import '../controls.dart';
import '../rust/api/chart.dart';
import '../rust/api/source.dart';
import '../rust/api/workspace.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'pdf_pages.dart';
import 'stored_name.dart';
import 'table_view_parts.dart' show MoreMenu;
import 'workspace_state.dart';

abstract final class SourceOpener {
  static Future<ProcessResult> Function(String program, List<String> arguments)
  run = Process.run;
}

String fileSize(int bytes) => switch (bytes) {
  < 1024 => '$bytes B',
  < 1024 * 1024 => '${(bytes / 1024).round()} KB',
  _ => '${(bytes / (1024 * 1024)).toStringAsFixed(1)} MB',
};

String kindOf(String name) {
  final dot = name.lastIndexOf('.');
  return switch (dot < 0 ? '' : name.substring(dot + 1).toLowerCase()) {
    'csv' => 'CSV',
    'tsv' => 'TSV',
    'md' || 'markdown' => 'Markdown',
    'txt' => 'Text',
    'pdf' => 'PDF',
    'png' || 'jpg' || 'jpeg' || 'gif' || 'webp' || 'heic' => 'Image',
    _ => 'File',
  };
}

IconData sourceIcon(String name, {required bool folder}) {
  if (folder) return LucideIcons.folder;
  return switch (kindOf(name)) {
    'CSV' || 'TSV' => LucideIcons.fileSpreadsheet,
    'Image' => LucideIcons.image,
    'Markdown' || 'Text' => LucideIcons.fileText,
    _ => LucideIcons.file,
  };
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
    _settle?.cancel();
    _name.dispose();
    _nameFocus.dispose();
    super.dispose();
  }

  void _renamed() {
    final name = _entry?.name;
    if (name != null && !_nameFocus.hasFocus) _name.show(name);
    if (mounted) setState(() {});
  }

  Future<void> _read() async {
    final read = ++_reads;
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

  Future<void> _system(List<String> arguments) async {
    try {
      final result = await SourceOpener.run(
        Platform.isMacOS ? 'open' : 'xdg-open',
        arguments,
      );
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
      final area = _state.areas.value
          .where(
            (area) =>
                _state.pages(area.id).value.any((known) => known.id == page),
          )
          .firstOrNull;
      if (area != null) _state.open.value = OpenPage(area.id, page);
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
          const Spacer(),
          GhostButton(
            label: 'Reload',
            icon: LucideIcons.refreshCw,
            onPressed: () => unawaited(_read()),
          ),
          if (_absolute case final path?)
            GhostButton(
              label: Platform.isMacOS ? 'Show in Finder' : 'Show in folder',
              onPressed: () => unawaited(_system(['-R', path])),
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
                  _state
                      .locateSource(
                        _source,
                        folder: entry?.kind == BridgeSourceKind.folder,
                      )
                      .then((_) => _read()),
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
                      hintText: 'Untitled',
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
        ..add(_listing(context, folder.entries))
        ..add(const SizedBox(height: WsMetrics.space3))
        ..add(
          Text('Liquid reads this folder and never writes it.', style: caption),
        );
      return _column(children);
    }
    final shown = file!;
    switch (shown.read) {
      case BridgeRead.tooLarge:
        children.add(
          _card(context, 'Liquid reads at most 50 MiB of a file.', shown.path),
        );
        return _column(children);
      case BridgeRead.unreadable:
        children.add(
          _card(context, 'Liquid can\'t read this file as text.', shown.path),
        );
        return _column(children);
      case BridgeRead.missing || BridgeRead.read:
        break;
    }
    switch (shown.shown) {
      case BridgeFileView.table:
        return _table(context, children, shown);
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
          Image.file(File(shown.path), key: const Key('source image')),
        );
      case BridgeFileView.pdf:
        children.add(_PdfSource(key: ValueKey(shown.path), path: shown.path));
      case BridgeFileView.other:
        children.add(
          _card(
            context,
            'Liquid shows no preview for this kind of file.',
            shown.path,
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

  Widget _column(List<Widget> children) => SingleChildScrollView(
    padding: const EdgeInsets.fromLTRB(
      WsMetrics.space7,
      28,
      WsMetrics.space7,
      WsMetrics.space8,
    ),
    child: Center(
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: WsMetrics.widthReading),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: children,
        ),
      ),
    ),
  );

  Widget _missing(BuildContext context, BridgeSource entry) {
    final ws = context.ws;
    final folder = entry.kind == BridgeSourceKind.folder;
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
                  onPressed: () => unawaited(
                    _state
                        .locateSource(_source, folder: folder)
                        .then((_) => _read()),
                  ),
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

  Widget _card(BuildContext context, String text, String path) {
    final ws = context.ws;
    final name = path.split(Platform.pathSeparator).last;
    return Container(
      key: const Key('source card'),
      padding: const EdgeInsets.all(WsMetrics.space4),
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
        border: Border.all(color: ws.border),
      ),
      child: Row(
        children: [
          Icon(LucideIcons.file, size: 22, color: ws.textFaint),
          const SizedBox(width: WsMetrics.space3),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(name),
                Text(
                  text,
                  style: TextStyle(
                    fontSize: WsMetrics.fsSmall,
                    color: ws.textFaint,
                  ),
                ),
              ],
            ),
          ),
          SecondaryButton(
            label: 'Open in default app',
            onPressed: () => unawaited(_system([path])),
          ),
        ],
      ),
    );
  }

  Widget _listing(BuildContext context, List<BridgeFileEntry> entries) {
    final ws = context.ws;
    final faint = TextStyle(fontSize: WsMetrics.fsCompact, color: ws.textFaint);
    final day = DateFormat.MMMd('en_US');
    Widget row(List<Widget> cells, {VoidCallback? onTap, Key? key}) => InkWell(
      key: key,
      onTap: onTap,
      child: Container(
        height: WsMetrics.rowTable,
        padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
        decoration: BoxDecoration(
          border: Border(bottom: BorderSide(color: ws.border)),
        ),
        child: Row(children: cells),
      ),
    );
    return Container(
      key: const Key('source listing'),
      clipBehavior: Clip.antiAlias,
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
        border: Border.all(color: ws.border),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          row([
            Expanded(child: Text('Name', style: faint)),
            SizedBox(width: 110, child: Text('Kind', style: faint)),
            SizedBox(
              width: 90,
              child: Text('Size', style: faint, textAlign: TextAlign.right),
            ),
            const SizedBox(width: WsMetrics.space4),
            SizedBox(width: 80, child: Text('Modified', style: faint)),
          ]),
          if (entries.isEmpty)
            Padding(
              padding: const EdgeInsets.all(WsMetrics.space4),
              child: Text('This folder is empty.', style: faint),
            ),
          for (final entry in entries)
            row(
              key: ValueKey('source entry ${entry.name}'),
              onTap: () => _open(
                _inside.isEmpty ? entry.name : '$_inside/${entry.name}',
              ),
              [
                Icon(
                  sourceIcon(entry.name, folder: entry.folder),
                  size: 14,
                  color: ws.textFaint,
                ),
                const SizedBox(width: WsMetrics.space2),
                Expanded(
                  child: Text(
                    entry.name,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
                SizedBox(
                  width: 110,
                  child: Text(entry.folder ? 'Folder' : kindOf(entry.name)),
                ),
                SizedBox(
                  width: 90,
                  child: Text(
                    entry.folder ? '—' : fileSize(entry.size.toInt()),
                    textAlign: TextAlign.right,
                  ),
                ),
                const SizedBox(width: WsMetrics.space4),
                SizedBox(
                  width: 80,
                  child: Text(
                    entry.modified == null
                        ? ''
                        : day.format(
                            DateTime.fromMillisecondsSinceEpoch(
                              entry.modified!.toInt(),
                            ),
                          ),
                  ),
                ),
              ],
            ),
        ],
      ),
    );
  }

  Widget _table(
    BuildContext context,
    List<Widget> head,
    BridgeSourceFile file,
  ) {
    final ws = context.ws;
    final faint = TextStyle(fontSize: WsMetrics.fsCompact, color: ws.textFaint);
    final caption = TextStyle(fontSize: WsMetrics.fsSmall, color: ws.textFaint);
    Widget line(List<String> cells, {bool header = false}) => Container(
      height: header ? 30 : WsMetrics.rowTable,
      decoration: BoxDecoration(
        color: header ? ws.subtle : null,
        border: Border(bottom: BorderSide(color: ws.border)),
      ),
      child: Row(
        children: [
          for (final (at, cell) in cells.indexed)
            Expanded(
              child: Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: WsMetrics.space3,
                ),
                child: Text(
                  cell,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  textAlign: file.columns[at].number
                      ? TextAlign.right
                      : TextAlign.left,
                  style: header ? faint : null,
                ),
              ),
            ),
        ],
      ),
    );
    final tail = <Widget>[
      const SizedBox(height: WsMetrics.space4),
      if (_inside.isEmpty) ...[
        Row(
          spacing: WsMetrics.space3,
          children: [
            _PagePicker(
              state: _state,
              onPage: (page) => unawaited(_useInBlock(page)),
            ),
            Flexible(
              child: Text(
                'Liquid reads this file and never writes it.',
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: caption,
              ),
            ),
          ],
        ),
        const SizedBox(height: WsMetrics.space4),
        Text('Used by', style: caption),
        const SizedBox(height: WsMetrics.space1),
        if (_users.isEmpty) Text('Nothing yet.', style: caption),
        for (final (page, _, title) in _users)
          InkWell(
            key: ValueKey('source user $page'),
            onTap: () {
              final area = _state.areas.value
                  .where(
                    (area) => _state
                        .pages(area.id)
                        .value
                        .any((known) => known.id == page),
                  )
                  .firstOrNull;
              if (area != null) _state.open.value = OpenPage(area.id, page);
            },
            child: SizedBox(
              height: 32,
              child: Row(
                children: [
                  Icon(LucideIcons.barChart3, size: 14, color: ws.textFaint),
                  const SizedBox(width: WsMetrics.space2),
                  Expanded(child: Text(title)),
                  Text('Chart · ${_pageTitle(page)}', style: caption),
                ],
              ),
            ),
          ),
      ],
    ];
    return CustomScrollView(
      slivers: [
        SliverPadding(
          padding: const EdgeInsets.fromLTRB(
            WsMetrics.space7,
            28,
            WsMetrics.space7,
            WsMetrics.space8,
          ),
          sliver: SliverCrossAxisConstrained(
            maxCrossAxisExtent: WsMetrics.widthReading,
            child: SliverList.list(
              children: [
                ...head,
                line([
                  for (final column in file.columns) column.name,
                ], header: true),
                for (final row in file.rows) line(row),
                ...tail,
              ],
            ),
          ),
        ),
      ],
    );
  }

  String _pageTitle(String page) {
    for (final area in _state.areas.value) {
      for (final known in _state.pages(area.id).value) {
        if (known.id == page) {
          return known.title.isEmpty ? 'Untitled' : known.title;
        }
      }
    }
    return 'Untitled';
  }
}

class SliverCrossAxisConstrained extends StatelessWidget {
  const SliverCrossAxisConstrained({
    super.key,
    required this.maxCrossAxisExtent,
    required this.child,
  });

  final double maxCrossAxisExtent;
  final Widget child;

  @override
  Widget build(BuildContext context) => SliverLayoutBuilder(
    builder: (context, constraints) {
      final extra = constraints.crossAxisExtent - maxCrossAxisExtent;
      return SliverPadding(
        padding: EdgeInsets.symmetric(horizontal: extra > 0 ? extra / 2 : 0),
        sliver: child,
      );
    },
  );
}

class _PagePicker extends StatelessWidget {
  const _PagePicker({required this.state, required this.onPage});

  final WorkspaceState state;
  final ValueChanged<String> onPage;

  @override
  Widget build(BuildContext context) => MenuAnchor(
    menuChildren: [
      for (final area in state.areas.value)
        for (final page in state.pages(area.id).value)
          MenuItemButton(
            key: ValueKey('use in ${page.id}'),
            onPressed: () => onPage(page.id),
            child: Text(page.title.isEmpty ? 'Untitled' : page.title),
          ),
    ],
    builder: (context, controller, _) => SecondaryButton(
      label: 'Use in a Block…',
      icon: LucideIcons.link,
      onPressed: () =>
          controller.isOpen ? controller.close() : controller.open(),
    ),
  );
}

class _PdfSource extends StatefulWidget {
  const _PdfSource({super.key, required this.path});

  final String path;

  @override
  State<_PdfSource> createState() => _PdfSourceState();
}

class _PdfSourceState extends State<_PdfSource> {
  PdfPages? _pdf;
  final _pages = <int, ui.Image>{};
  bool _failed = false;

  @override
  void initState() {
    super.initState();
    unawaited(_open());
  }

  Future<void> _open() async {
    try {
      final pdf = await PdfPages.open(await File(widget.path).readAsBytes());
      if (!mounted) {
        await pdf.close();
        return;
      }
      setState(() => _pdf = pdf);
      for (var at = 0; at < pdf.sizes.length && mounted; at++) {
        final image = await pdf.render(at, 1400);
        if (!mounted) return;
        setState(() => _pages[at] = image);
      }
    } on Object {
      if (mounted) setState(() => _failed = true);
    }
  }

  @override
  void dispose() {
    unawaited(_pdf?.close());
    for (final image in _pages.values) {
      image.dispose();
    }
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    if (_failed) {
      return Text(
        'Liquid could not show this PDF.',
        style: TextStyle(color: ws.textFaint),
      );
    }
    final pdf = _pdf;
    if (pdf == null) return const SizedBox(height: 200);
    return Column(
      key: const Key('source pdf'),
      spacing: WsMetrics.space3,
      children: [
        for (final (at, size) in pdf.sizes.indexed)
          AspectRatio(
            aspectRatio: size.width / size.height,
            child: _pages[at] == null
                ? ColoredBox(color: ws.subtle)
                : RawImage(image: _pages[at], fit: BoxFit.contain),
          ),
      ],
    );
  }
}
