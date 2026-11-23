import 'dart:async';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';

import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'pdf_pages.dart';

class SourcePdf extends StatefulWidget {
  const SourcePdf({super.key, required this.path});

  final String path;

  @override
  State<SourcePdf> createState() => _PdfSourceState();
}

class _PdfSourceState extends State<SourcePdf> {
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
