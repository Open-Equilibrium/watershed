import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../rust/api/media.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'media_block.dart';

const maxCellFiles = 100;

const _chipGap = WsMetrics.space1;
const _chipPadding = 6.0;
const _chipIcon = 13.0;

IconData _iconOf(BridgeMedia file) =>
    file.mediaType.startsWith('image/') ? LucideIcons.image : LucideIcons.file;

class FileChips extends StatelessWidget {
  const FileChips(this.files, {super.key});

  final List<BridgeMedia> files;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final style = TextStyle(fontSize: WsMetrics.fsCompact, color: ws.text);
    return LayoutBuilder(
      builder: (context, constraints) {
        double widthOf(BridgeMedia file) {
          final painter = TextPainter(
            text: TextSpan(text: file.name, style: style),
            textDirection: TextDirection.ltr,
            maxLines: 1,
          )..layout(maxWidth: 150);
          final width = painter.width;
          painter.dispose();
          return 2 * _chipPadding + _chipIcon + 4 + width + _chipGap;
        }

        const more = 28.0;
        var used = 0.0;
        var shown = 0;
        for (final (index, file) in files.indexed) {
          final width = widthOf(file);
          final rest = index == files.length - 1 ? 0 : more;
          if (shown > 0 && used + width + rest > constraints.maxWidth) break;
          used += width;
          shown++;
        }
        return ClipRect(
          child: Row(
            children: [
              for (final file in files.take(shown))
                Flexible(
                  child: Padding(
                    padding: const EdgeInsets.only(right: _chipGap),
                    child: Container(
                      key: ValueKey('file chip ${file.name}'),
                      constraints: const BoxConstraints(maxWidth: 150),
                      height: 22,
                      padding: const EdgeInsets.symmetric(
                        horizontal: _chipPadding,
                      ),
                      decoration: BoxDecoration(
                        color: ws.subtle,
                        borderRadius: BorderRadius.circular(5),
                        border: Border.all(color: ws.border),
                      ),
                      child: Row(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          Icon(
                            _iconOf(file),
                            size: _chipIcon,
                            color: ws.textMuted,
                          ),
                          const SizedBox(width: 4),
                          Flexible(
                            child: Text(
                              file.name,
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                              style: style,
                            ),
                          ),
                        ],
                      ),
                    ),
                  ),
                ),
              if (shown < files.length)
                Text(
                  '+${files.length - shown}',
                  style: TextStyle(
                    fontSize: WsMetrics.fsCompact,
                    color: ws.textFaint,
                  ),
                ),
            ],
          ),
        );
      },
    );
  }
}

class FilesPicker extends StatefulWidget {
  const FilesPicker({
    super.key,
    required this.files,
    required this.onSave,
    required this.onRemove,
    required this.onAdd,
    required this.onClose,
    required this.onTab,
    this.onKeys,
  });

  final List<BridgeMedia> files;
  final ValueChanged<BridgeMedia> onSave;
  final ValueChanged<int> onRemove;
  final VoidCallback onAdd;
  final VoidCallback onClose;
  final ValueChanged<bool> onTab;

  final void Function(KeyEventResult Function(KeyEvent event)? keys)? onKeys;

  static const _labelHeight = 26.0;
  static const _itemHeight = 30.0;
  static const _width = 300.0;

  static double heightFor(int files) =>
      2 * WsMetrics.space2 +
      (files > 0 ? _labelHeight + files * _itemHeight + 9 : 0) +
      _itemHeight;

  @override
  State<FilesPicker> createState() => _FilesPickerState();
}

class _FilesPickerState extends State<FilesPicker> {
  late final _focus = FocusNode(debugLabel: 'Files picker', onKeyEvent: _key);
  final _list = ScrollController();
  var _chosen = 0;

  void _choose(int index) {
    setState(() => _chosen = index);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted || !_list.hasClients || index >= widget.files.length) return;
      final position = _list.position;
      final top = index * FilesPicker._itemHeight;
      final bottom = top + FilesPicker._itemHeight - position.viewportDimension;
      if (top < position.pixels) position.jumpTo(top);
      if (bottom > position.pixels) position.jumpTo(bottom);
    });
  }

  @override
  void initState() {
    super.initState();
    widget.onKeys?.call((event) => _key(_focus, event));
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _focus.requestFocus();
    });
  }

  @override
  void dispose() {
    widget.onKeys?.call(null);
    _focus.dispose();
    _list.dispose();
    super.dispose();
  }

  int get _count => widget.files.length + 1;

  void _take(int index) {
    if (index < widget.files.length) {
      widget.onSave(widget.files[index]);
    } else {
      widget.onAdd();
    }
  }

  KeyEventResult _key(FocusNode _, KeyEvent event) {
    if (event is KeyUpEvent) return KeyEventResult.ignored;
    switch (event.logicalKey) {
      case LogicalKeyboardKey.arrowDown || LogicalKeyboardKey.arrowUp:
        final by = event.logicalKey == LogicalKeyboardKey.arrowDown ? 1 : -1;
        _choose((_chosen + by) % _count);
      case LogicalKeyboardKey.enter || LogicalKeyboardKey.numpadEnter:
        if (event is! KeyRepeatEvent) _take(_chosen);
      case LogicalKeyboardKey.backspace || LogicalKeyboardKey.delete
          when _chosen < widget.files.length:
        if (event is! KeyRepeatEvent) widget.onRemove(_chosen);
      case LogicalKeyboardKey.escape:
        widget.onClose();
      case LogicalKeyboardKey.tab:
        widget.onTab(HardwareKeyboard.instance.isShiftPressed);
      default:
        return KeyEventResult.ignored;
    }
    return KeyEventResult.handled;
  }

  @override
  void didUpdateWidget(FilesPicker old) {
    super.didUpdateWidget(old);
    if (_chosen >= _count) _chosen = _count - 1;
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final files = widget.files;
    Widget action(IconData icon, Key key, VoidCallback onTap) => MouseRegion(
      cursor: SystemMouseCursors.click,
      child: GestureDetector(
        key: key,
        onTap: onTap,
        child: Padding(
          padding: const EdgeInsets.all(4),
          child: Icon(icon, size: 14, color: ws.textMuted),
        ),
      ),
    );
    Widget item(int index, List<Widget> children) => MouseRegion(
      onEnter: (_) => setState(() => _chosen = index),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: () => _take(index),
        child: Container(
          height: FilesPicker._itemHeight,
          padding: const EdgeInsets.only(left: WsMetrics.space2, right: 2),
          decoration: BoxDecoration(
            color: index == _chosen ? ws.hover : null,
            borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
          ),
          child: Row(children: children),
        ),
      ),
    );
    return Focus(
      focusNode: _focus,
      child: Container(
        key: const Key('files picker'),
        width: FilesPicker._width,
        padding: const EdgeInsets.all(WsMetrics.space2),
        decoration: BoxDecoration(
          color: ws.surface,
          borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
          boxShadow: ws.shadow,
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            if (files.isNotEmpty) ...[
              SizedBox(
                height: FilesPicker._labelHeight,
                child: Align(
                  alignment: Alignment.centerLeft,
                  child: Padding(
                    padding: const EdgeInsets.only(left: WsMetrics.space2),
                    child: Text(
                      files.length == 1 ? '1 file' : '${files.length} files',
                      style: TextStyle(
                        fontSize: WsMetrics.fsCompact,
                        color: ws.textFaint,
                      ),
                    ),
                  ),
                ),
              ),
              Flexible(
                child: ListView.builder(
                  controller: _list,
                  shrinkWrap: true,
                  itemExtent: FilesPicker._itemHeight,
                  itemCount: files.length,
                  itemBuilder: (context, index) {
                    final file = files[index];
                    return item(index, [
                      Icon(_iconOf(file), size: 14, color: ws.textMuted),
                      const SizedBox(width: WsMetrics.space2),
                      Expanded(
                        child: Text(
                          '${file.name} · ${sizeLabel(file.size)}',
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(
                            fontSize: WsMetrics.fsUi,
                            color: ws.text,
                          ),
                        ),
                      ),
                      if (index == _chosen) ...[
                        action(
                          LucideIcons.download,
                          ValueKey('file save ${file.name}'),
                          () => widget.onSave(file),
                        ),
                        action(
                          LucideIcons.x,
                          ValueKey('file remove ${file.name}'),
                          () => widget.onRemove(index),
                        ),
                      ],
                    ]);
                  },
                ),
              ),
              Divider(height: 9, thickness: 1, color: ws.border),
            ],
            item(files.length, [
              Icon(LucideIcons.plus, size: 14, color: ws.textMuted),
              const SizedBox(width: WsMetrics.space2),
              Text(
                'Add files…',
                key: const Key('add files'),
                style: TextStyle(fontSize: WsMetrics.fsUi, color: ws.text),
              ),
            ]),
          ],
        ),
      ),
    );
  }
}
