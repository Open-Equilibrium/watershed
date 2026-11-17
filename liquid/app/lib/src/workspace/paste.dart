import 'dart:convert';

import 'package:flutter/services.dart';

import 'code_block.dart';

/// What the clipboard holds, as far as a paste into a Text Block tells apart: copied files,
/// an image, code an editor marked, or plain text (ADR-0305).
class Pasted {
  const Pasted({
    this.files = const [],
    this.image,
    this.code,
    this.language = '',
    this.text,
  });

  /// Files copied in Finder, by path.
  final List<String> files;

  /// An image, as PNG.
  final Uint8List? image;

  /// Code, with its language's stored name; empty for plain text.
  final String? code;
  final String language;

  final String? text;
}

/// Reads the clipboard: on macOS through Liquid's own reader in its window, which knows
/// images, files and editors' marks (ADR-0305); elsewhere its plain text only, as Flutter
/// reads it. Tests replace it.
Future<Pasted> Function() readClipboard = () async {
  try {
    final read = await const MethodChannel('liquid/clipboard')
        .invokeMapMethod<String, Object?>('read');
    if (read != null) return pastedFrom(read);
  } on MissingPluginException {
    // No reader on this platform.
  }
  return Pasted(text: (await Clipboard.getData(Clipboard.kTextPlain))?.text);
};

/// Whether `event` is ⌘V, which Liquid's own paste takes (ADR-0305).
bool isPaste(KeyEvent event) {
  final keys = HardwareKeyboard.instance;
  return event.logicalKey == LogicalKeyboardKey.keyV &&
      keys.isMetaPressed &&
      !keys.isShiftPressed &&
      !keys.isControlPressed &&
      !keys.isAltPressed;
}

/// What the native reader's answer holds: `files`, `image` (PNG), `vscode` (the editor data
/// VS Code and its forks put beside copied code), `html` and `text`.
Pasted pastedFrom(Map<String, Object?> read) {
  final text = read['text'] as String?;
  final files = (read['files'] as List?)?.cast<String>() ?? const [];
  if (files.isNotEmpty) return Pasted(files: files, text: text);
  // Excel, Word and Keynote put a picture beside copied text, so the text wins; a browser
  // copies an image with no text but its address.
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

/// The language VS Code names in its editor data, if the picker knows it.
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

/// Whether `html` is a single `pre` element, as code copied from a page or many editors is.
bool _onlyPre(String html) {
  final bare = html.replaceAll(_wrapper, '').trim().toLowerCase();
  return bare.startsWith('<pre') &&
      bare.endsWith('</pre>') &&
      '<pre'.allMatches(bare).length == 1;
}
