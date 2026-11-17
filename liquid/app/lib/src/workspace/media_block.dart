import 'dart:async';
import 'dart:io';
import 'dart:math' as math;
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:file_selector/file_selector.dart';
import 'package:flutter/material.dart';

import '../rust/api/media.dart';
import '../rust/api/workspace.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'anchored_menu.dart';
import 'follow.dart';
import 'page_views.dart';
import 'workspace_state.dart';

/// The largest file a Media Block takes: 5 GiB, as the core caps it (ADR-0305).
const maxAttachment = 5 << 30;

/// The largest image Liquid shows, as the bridge's `MAX_SHOWN`; a larger one shows as a file.
const maxShown = 100 << 20;

/// A file to add: on disk, read in Rust, or pasted bytes.
class MediaSource {
  const MediaSource.file(this.name, String this.path, this.size) : bytes = null;
  MediaSource.bytes(this.name, Uint8List this.bytes)
    : path = null,
      size = bytes.length;

  final String name;
  final String? path;
  final Uint8List? bytes;
  final int size;
}

/// What the system offers a Media Block: its file dialogs. Tests replace them.
abstract final class MediaPlatform {
  /// Asks for files to add; none if the person cancelled.
  static Future<List<MediaSource>> Function() pickFiles = () async => [
    for (final file in await openFiles())
      MediaSource.file(file.name, file.path, await File(file.path).length()),
  ];

  /// Asks where to save a copy of `name`; none if the person cancelled.
  static Future<String?> Function(String name) saveLocation = (name) async =>
      (await getSaveLocation(suggestedName: name))?.path;
}

/// The media types Liquid knows by a file's extension; the core stores the type with the
/// Block, so export and sync keep it.
const _mediaTypes = {
  'png': 'image/png',
  'jpg': 'image/jpeg',
  'jpeg': 'image/jpeg',
  'gif': 'image/gif',
  'webp': 'image/webp',
  'bmp': 'image/bmp',
  'heic': 'image/heic',
  'pdf': 'application/pdf',
  'txt': 'text/plain',
  'csv': 'text/csv',
  'md': 'text/markdown',
  'eml': 'message/rfc822',
  'zip': 'application/zip',
  'mov': 'video/quicktime',
  'mp4': 'video/mp4',
  'mp3': 'audio/mpeg',
};

/// The images Flutter decodes, which show as images (ADR-0305).
const _shownImages = {
  'image/png',
  'image/jpeg',
  'image/gif',
  'image/webp',
  'image/bmp',
};

String _extension(String name) {
  final dot = name.lastIndexOf('.');
  return dot <= 0 ? '' : name.substring(dot + 1).toLowerCase();
}

/// The media type of a file named `name`; empty if Liquid does not know it.
String mediaTypeOf(String name) => _mediaTypes[_extension(name)] ?? '';

/// A size as Finder shows it: decimal units, one decimal below 10.
String sizeLabel(int bytes) {
  const units = ['bytes', 'KB', 'MB', 'GB', 'TB'];
  var value = bytes.toDouble();
  var unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit++;
  }
  if (unit == 0) return '$bytes ${bytes == 1 ? 'byte' : 'bytes'}';
  final digits = value < 10 ? 1 : 0;
  return '${value.toStringAsFixed(digits)} ${units[unit]}';
}

/// What kind of file it is, as the line shows it: its extension, or "File".
String kindLabel(String name, String mediaType) {
  final extension = _extension(name).toUpperCase();
  if (extension.isEmpty) return 'File';
  return mediaType.startsWith('image/') ? '$extension image' : extension;
}

/// A file going in, shown where its Media Block will be until it is.
class PendingUpload {
  PendingUpload(this.name, this.size, this.after);

  final String name;
  final int size;

  /// The View it follows, or none at the Page's start.
  final String? after;

  final written = ValueNotifier<int>(0);
}

/// Adds `sources` as Media Blocks after the View `after`, or first, in their order, each
/// shown with its progress until it is in. A file over 5 GiB is refused before it is read.
/// The empty line `replacing` goes once the first is in, unless it ends the Page.
Future<void> addMedia(
  PageViews views,
  List<MediaSource> sources, {
  required String? after,
  BridgeView? replacing,
}) async {
  final state = views.state;
  var follows = after;
  for (final source in sources) {
    if (source.size > maxAttachment) {
      // After the writes before it, whose success would take the problem away.
      await state.settled;
      state.report(
        '“${source.name}” is ${sizeLabel(source.size)}; a file can be at most 5 GiB.',
      );
      continue;
    }
    final pending = PendingUpload(source.name, source.size, follows);
    views.addUpload(pending);
    try {
      final String attachment;
      if (source.bytes case final bytes?) {
        attachment = await state.workspace.addBytes(bytes: bytes);
      } else {
        String? named;
        await for (final step in state.workspace.addFile(path: source.path!)) {
          pending.written.value = step.written;
          named = step.attachment ?? named;
        }
        attachment = named!;
      }
      final added = await views.write(
        mutation(
          BridgeMutationKind.addMediaBlock,
          page: views.page,
          after: follows,
          text: source.name,
          mediaType: mediaTypeOf(source.name),
          attachment: attachment,
        ),
        const [],
      );
      follows = added.view;
    } on Object catch (error) {
      state.report('Liquid did not add “${source.name}”: $error');
      continue;
    } finally {
      views.removeUpload(pending);
    }
    if (replacing case final line?) {
      replacing = null;
      final index = views.indexOf(line.id);
      if (index != null && index < (views.count ?? 0) - 1) {
        unawaited(
          views
              .write(
                mutation(
                  BridgeMutationKind.removeView,
                  page: views.page,
                  view: line.id,
                ),
                [line.block],
              )
              .then<void>((_) {}, onError: (Object _) {}),
        );
      }
    }
  }
}

/// A file going in: its name, and how much of it is in (ADR-0305).
class UploadRow extends StatelessWidget {
  const UploadRow(this.upload, {super.key});

  final PendingUpload upload;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return ValueListenableBuilder(
      valueListenable: upload.written,
      builder: (context, written, _) => _FileLine(
        name: upload.name,
        trailing: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(
              '${sizeLabel(written)} of ${sizeLabel(upload.size)}',
              style: TextStyle(
                fontSize: WsMetrics.fsSmall,
                color: ws.textFaint,
              ),
            ),
            const SizedBox(width: WsMetrics.space2),
            SizedBox(
              width: 120,
              child: LinearProgressIndicator(
                value: upload.size == 0 ? 1 : written / upload.size,
                minHeight: 4,
                color: ws.accent,
                backgroundColor: ws.border,
                borderRadius: BorderRadius.circular(2),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// A file on one line: its icon, its name and, at the line's end, `trailing`.
class _FileLine extends StatelessWidget {
  const _FileLine({required this.name, required this.trailing});

  final String name;
  final Widget trailing;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      margin: const EdgeInsets.symmetric(vertical: WsMetrics.space1),
      padding: const EdgeInsets.fromLTRB(
        WsMetrics.space2,
        WsMetrics.space1 + 2,
        WsMetrics.space3,
        WsMetrics.space1 + 2,
      ),
      decoration: BoxDecoration(
        border: Border.all(color: ws.border),
        borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
      ),
      child: Row(
        children: [
          Container(
            width: 24,
            height: 24,
            alignment: Alignment.center,
            decoration: BoxDecoration(
              color: ws.subtle,
              borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
            ),
            child: Icon(LucideIcons.file, size: 14, color: ws.textMuted),
          ),
          const SizedBox(width: WsMetrics.space3),
          Expanded(
            child: Text(
              name,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(fontSize: WsMetrics.fsUi, color: ws.text),
            ),
          ),
          const SizedBox(width: WsMetrics.space3),
          trailing,
        ],
      ),
    );
  }
}

/// A Media Block: an image at its own size, never wider than the column, or a file on one
/// line; hovering shows Save as… and ⋯ with Delete (ADR-0305).
class MediaBlock extends StatefulWidget {
  const MediaBlock({super.key, required this.views, required this.view});

  final PageViews views;
  final BridgeView view;

  @override
  State<MediaBlock> createState() => _MediaBlockState();
}

class _MediaBlockState extends State<MediaBlock> {
  BridgeMedia? _media;

  /// The image's bytes and its size in pixels, once read and decoded; none for a file.
  (Uint8List, Size)? _image;
  StreamSubscription<BridgeChange>? _followed;
  bool _hovered = false;
  final _more = OverlayPortalController();

  WorkspaceState get _state => widget.views.state;

  @override
  void initState() {
    super.initState();
    _followed = follow(
      _state.workspace,
      BridgeResource(
        kind: BridgeResourceKind.block,
        page: widget.views.page,
        block: widget.view.block,
      ),
    ).listen((_) => unawaited(_read()), onError: _report);
  }

  @override
  void dispose() {
    unawaited(_followed?.cancel());
    super.dispose();
  }

  void _report(Object error) {
    if (mounted) _state.readFailed(error);
  }

  Future<void> _read() async {
    final BridgeMedia? media;
    try {
      media = (await _state.read(
        query(
          BridgeQueryKind.media,
          page: widget.views.page,
          block: widget.view.block,
        ),
      )).media;
    } on Object catch (error) {
      _report(error);
      return;
    }
    if (!mounted || media == null || media == _media) return;
    setState(() => _media = media);
    if (_shownImages.contains(media.mediaType) && media.size <= maxShown) {
      final image = await _decode(media.attachment);
      if (mounted && _media == media) setState(() => _image = image);
    }
  }

  /// The image's bytes and pixel size; none if it does not decode, so it shows as a file.
  Future<(Uint8List, Size)?> _decode(String attachment) async {
    try {
      final bytes = await _state.workspace.readAttachment(
        attachment: attachment,
      );
      final buffer = await ui.ImmutableBuffer.fromUint8List(bytes);
      final descriptor = await ui.ImageDescriptor.encoded(buffer);
      final size = Size(
        descriptor.width.toDouble(),
        descriptor.height.toDouble(),
      );
      descriptor.dispose();
      buffer.dispose();
      return (bytes, size);
    } on Object {
      return null;
    }
  }

  Future<void> _saveAs() async {
    _more.hide();
    final media = _media;
    if (media == null) return;
    final path = await MediaPlatform.saveLocation(media.name);
    if (path == null) return;
    try {
      await _state.workspace.saveAttachment(
        attachment: media.attachment,
        path: path,
      );
    } on Object catch (error) {
      _state.report('Liquid did not save “${media.name}”: $error');
    }
  }

  void _delete() {
    _more.hide();
    unawaited(
      widget.views
          .write(
            mutation(
              BridgeMutationKind.removeView,
              page: widget.views.page,
              view: widget.view.id,
            ),
            [widget.view.block],
          )
          .then<void>((_) {}, onError: (Object _) {}),
    );
  }

  Widget _button(
    BuildContext context,
    IconData icon, {
    String? label,
    required VoidCallback onTap,
    required Key key,
  }) {
    final ws = context.ws;
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      child: GestureDetector(
        key: key,
        onTap: onTap,
        child: Container(
          height: 24,
          padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space2),
          alignment: Alignment.center,
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(icon, size: 14, color: ws.textMuted),
              if (label != null) ...[
                const SizedBox(width: WsMetrics.space2),
                Text(
                  label,
                  style: TextStyle(
                    fontSize: WsMetrics.fsSmall,
                    color: ws.textMuted,
                  ),
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }

  Widget _menuItem(
    BuildContext context,
    IconData icon,
    String label,
    VoidCallback onTap, {
    bool danger = false,
  }) {
    final ws = context.ws;
    final color = danger ? ws.danger : ws.text;
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      child: GestureDetector(
        key: ValueKey('media $label'),
        onTap: onTap,
        child: Container(
          height: 30,
          padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space2),
          child: Row(
            children: [
              Icon(icon, size: 14, color: color),
              const SizedBox(width: WsMetrics.space2),
              Text(
                label,
                style: TextStyle(fontSize: WsMetrics.fsUi, color: color),
              ),
            ],
          ),
        ),
      ),
    );
  }

  Widget _bar(BuildContext context, {required bool file}) {
    final ws = context.ws;
    return Container(
      padding: const EdgeInsets.all(2),
      decoration: BoxDecoration(
        color: ws.surface,
        borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
        boxShadow: ws.shadow,
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          _button(
            context,
            LucideIcons.download,
            label: file ? 'Save as…' : null,
            onTap: () => unawaited(_saveAs()),
            key: const Key('media save'),
          ),
          AnchoredMenu(
            controller: _more,
            onTapOutside: (_) => setState(_more.hide),
            menu: (context) => Container(
              width: 170,
              padding: const EdgeInsets.all(WsMetrics.space1 + 2),
              decoration: BoxDecoration(
                color: ws.surface,
                borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
                boxShadow: ws.shadow,
              ),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  _menuItem(
                    context,
                    LucideIcons.download,
                    'Save as…',
                    () => unawaited(_saveAs()),
                  ),
                  Divider(height: WsMetrics.space2, color: ws.border),
                  _menuItem(
                    context,
                    LucideIcons.trash,
                    'Delete',
                    _delete,
                    danger: true,
                  ),
                ],
              ),
            ),
            child: _button(
              context,
              LucideIcons.ellipsis,
              onTap: () => setState(_more.show),
              key: const Key('media more'),
            ),
          ),
        ],
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final media = _media;
    if (media == null) return const SizedBox(height: WsMetrics.lhProse);
    final image = _image;
    final Widget shown;
    if (image case (final bytes, final pixels)) {
      shown = LayoutBuilder(
        builder: (context, constraints) {
          final width = math.min(pixels.width, constraints.maxWidth);
          final ratio = MediaQuery.devicePixelRatioOf(context);
          return Align(
            alignment: Alignment.centerLeft,
            child: ClipRRect(
              borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
              child: Image.memory(
                bytes,
                key: const Key('media image'),
                width: width,
                height: width * pixels.height / pixels.width,
                // Decoded at the size shown, so memory stays bounded.
                cacheWidth: (width * ratio).round(),
                fit: BoxFit.contain,
                gaplessPlayback: true,
              ),
            ),
          );
        },
      );
    } else {
      shown = _FileLine(
        name: media.name,
        trailing: Text(
          '${sizeLabel(media.size)} · ${kindLabel(media.name, media.mediaType)}',
          style: TextStyle(fontSize: WsMetrics.fsSmall, color: ws.textFaint),
        ),
      );
    }
    return MouseRegion(
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: WsMetrics.space1),
        child: Stack(
          children: [
            shown,
            if (_hovered || _more.isShowing)
              Positioned(
                top: image == null ? WsMetrics.space1 + 2 : WsMetrics.space2,
                right: image == null ? WsMetrics.space2 : WsMetrics.space2,
                child: _bar(context, file: image == null),
              ),
          ],
        ),
      ),
    );
  }
}
