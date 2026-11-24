import 'dart:async';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';

import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'pdf_pages.dart';

class SourcePdf extends StatefulWidget {
  const SourcePdf({super.key, required this.bytes});

  final Uint8List bytes;

  @override
  State<SourcePdf> createState() => _PdfSourceState();
}

class _PdfSourceState extends State<SourcePdf> {
  PdfPages? _pdf;
  bool _failed = false;

  @override
  void initState() {
    super.initState();
    unawaited(_open());
  }

  Future<void> _open() async {
    try {
      final pdf = await PdfPages.open(widget.bytes);
      if (!mounted) {
        await pdf.close();
        return;
      }
      setState(() => _pdf = pdf);
    } on Object {
      if (mounted) setState(() => _failed = true);
    }
  }

  @override
  void dispose() {
    unawaited(_pdf?.close());
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
    return ListView.builder(
      key: const Key('source pdf'),
      itemCount: pdf.sizes.length,
      itemBuilder: (context, at) => Padding(
        padding: const EdgeInsets.only(bottom: WsMetrics.space3),
        child: _PdfPage(
          pdf: pdf,
          at: at,
          size: pdf.sizes[at],
          onFailed: () => setState(() => _failed = true),
        ),
      ),
    );
  }
}

class _PdfPage extends StatefulWidget {
  const _PdfPage({
    required this.pdf,
    required this.at,
    required this.size,
    required this.onFailed,
  });

  final PdfPages pdf;
  final int at;
  final ui.Size size;
  final VoidCallback onFailed;

  @override
  State<_PdfPage> createState() => _PdfPageState();
}

class _PdfPageState extends State<_PdfPage> {
  ui.Image? _image;

  @override
  void initState() {
    super.initState();
    unawaited(_render());
  }

  Future<void> _render() async {
    try {
      final image = await widget.pdf.render(widget.at, 1400);
      if (!mounted) {
        image.dispose();
        return;
      }
      setState(() => _image = image);
    } on Object {
      if (mounted) widget.onFailed();
    }
  }

  @override
  void dispose() {
    _image?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => AspectRatio(
    aspectRatio: widget.size.width / widget.size.height,
    child: _image == null
        ? ColoredBox(color: context.ws.subtle)
        : RawImage(image: _image, fit: BoxFit.contain),
  );
}
