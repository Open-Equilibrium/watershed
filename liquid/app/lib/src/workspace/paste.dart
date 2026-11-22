import 'dart:convert';

import 'package:flutter/services.dart';

import 'code_block.dart';

class Pasted {
  const Pasted({
    this.files = const [],
    this.image,
    this.code,
    this.language = '',
    this.text,
  });

  final List<String> files;

  final Uint8List? image;

  final String? code;
  final String language;

  final String? text;
}

Future<void> Function(String text) writeClipboard = (text) =>
    Clipboard.setData(ClipboardData(text: text));

Future<Pasted> Function() readClipboard = () async {
  try {
    final read = await const MethodChannel('liquid/clipboard')
        .invokeMapMethod<String, Object?>('read');
    if (read != null) return pastedFrom(read);
  } on MissingPluginException catch (_) {}
  return Pasted(text: (await Clipboard.getData(Clipboard.kTextPlain))?.text);
};

bool isPaste(KeyEvent event) {
  final keys = HardwareKeyboard.instance;
  return event.logicalKey == LogicalKeyboardKey.keyV &&
      keys.isMetaPressed &&
      !keys.isShiftPressed &&
      !keys.isControlPressed &&
      !keys.isAltPressed;
}

Pasted pastedFrom(Map<String, Object?> read) {
  final text = read['text'] as String?;
  final files = (read['files'] as List?)?.cast<String>() ?? const [];
  if (files.isNotEmpty) return Pasted(files: files, text: text);
  final address =
      text != null && RegExp(r'^https?://\S+$').hasMatch(text.trim());
  if (read['image'] case final Uint8List image
      when text == null || text.trim().isEmpty || address) {
    return Pasted(image: image, text: text);
  }
  if (text != null && text.isNotEmpty) {
    if (read['vscode'] case final String data) {
      return Pasted(code: text, language: _vscodeLanguage(data), text: text);
    }
    if (read['html'] case final String html when _onlyPre(html)) {
      return Pasted(code: text, text: text);
    }
  }
  return Pasted(text: text);
}

String _vscodeLanguage(String data) {
  try {
    final mode = (jsonDecode(data) as Map)['mode'];
    final name = switch (mode) {
      'shellscript' => 'bash',
      'typescriptreact' => 'typescript',
      'javascriptreact' => 'javascript',
      _ => mode,
    };
    return codeLanguages.any((known) => known.$1 == name) ? name as String : '';
  } on Object {
    return '';
  }
}

final _wrapper = RegExp(
  r'<!--.*?-->|<meta[^>]*>|</?(html|head|body)[^>]*>',
  caseSensitive: false,
  dotAll: true,
);

bool _onlyPre(String html) {
  final bare = html.replaceAll(_wrapper, '').trim().toLowerCase();
  return bare.startsWith('<pre') &&
      bare.endsWith('</pre>') &&
      '<pre'.allMatches(bare).length == 1;
}
