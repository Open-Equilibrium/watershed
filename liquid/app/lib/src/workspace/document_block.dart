import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../rust/api/media.dart';
import '../rust/api/workspace.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../whiteboard/board_controller.dart';
import '../whiteboard/document_pages.dart';
import 'media_block.dart';
import 'page_views.dart';
import 'pdf_pages.dart';
import 'whiteboard_block.dart';

class DocumentBlock extends StatefulWidget {
  const DocumentBlock({
    super.key,
    required this.views,
    required this.view,
    required this.media,
    required this.onUnreadable,
  });

  final PageViews views;
  final BridgeView view;
  final BridgeMedia media;
  final VoidCallback onUnreadable;

  @override
  State<DocumentBlock> createState() => _DocumentBlockState();
}

class _DocumentBlockState extends State<DocumentBlock> {
  late final BoardController _board = widget.views.boardFor(widget.view);
  var _placed = false;

  @override
  void initState() {
    super.initState();
    unawaited(_open());
  }

  Future<void> _open() async {
    final state = widget.views.state;
    try {
      final bytes = await state.workspace.readAttachment(
        attachment: widget.media.attachment,
      );
      final pdf = await PdfPages.open(bytes);
      if (!mounted) {
        await pdf.close();
        return;
      }
      _board
        ..title = widget.media.name
        ..pages = DocumentPages.pdf(pdf, onDrawn: _board.notifyListeners);
      setState(() {});
    } on PdfUnreadable {
      if (mounted) widget.onUnreadable();
    } on Object catch (error) {
      if (mounted) {
        state.report('Liquid could not show “${widget.media.name}”: $error');
      }
    }
  }

  @override
  void dispose() {
    widget.views.releaseBoard(widget.view);
    super.dispose();
  }

  void _place(double width) {
    final sheets = _board.sheets;
    if (_placed || sheets.isEmpty || width <= 0) return;
    _placed = true;
    final widest = sheets.map((sheet) => sheet.width).reduce(math.max);
    _board
      ..zoom = ((width - 2 * DocumentPages.gap) / widest).clamp(
        BoardController.minZoom,
        BoardController.maxZoom,
      )
      ..origin = Offset(-width / 2 / _board.zoom, -DocumentPages.gap);
  }

  @override
  Widget build(BuildContext context) {
    if (_board.pages == null) {
      return Container(
        key: const Key('document loading'),
        height: documentHeight,
        margin: const EdgeInsets.symmetric(vertical: 4),
        decoration: BoxDecoration(
          color: context.ws.subtle,
          borderRadius: BorderRadius.circular(10),
        ),
      );
    }
    return LayoutBuilder(
      builder: (context, constraints) {
        _place(constraints.maxWidth);
        return DrawingFrame(
          views: widget.views,
          view: widget.view,
          name: 'document',
          height: documentHeight,
          buttons: [
            BarButton(
              key: const Key('document save'),
              icon: LucideIcons.download,
              label: 'Save as…',
              showLabel: true,
              onTap: () =>
                  unawaited(saveCopy(widget.views.state, widget.media)),
            ),
          ],
        );
      },
    );
  }
}

class ImageInk extends StatefulWidget {
  const ImageInk({
    super.key,
    required this.views,
    required this.view,
    required this.pixels,
    required this.width,
  });

  final PageViews views;
  final BridgeView view;

  final Size pixels;
  final double width;

  @override
  State<ImageInk> createState() => _ImageInkState();
}

class _ImageInkState extends State<ImageInk> {
  late final BoardController _board = widget.views.boardFor(widget.view);

  @override
  void dispose() {
    widget.views.releaseBoard(widget.view);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final pixels = widget.pixels;
    _board
      ..title = 'Image'
      ..fixed = true
      ..sheets = [Rect.fromLTWH(0, 0, pixels.width, pixels.height)]
      ..zoom = widget.width / pixels.width
      ..origin = Offset.zero;
    return DrawingFrame(
      views: widget.views,
      view: widget.view,
      name: 'image ink',
      height: widget.width * pixels.height / pixels.width,
      framed: false,
      bar: false,
    );
  }
}
