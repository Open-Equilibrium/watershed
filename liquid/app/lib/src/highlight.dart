import 'package:flutter/widgets.dart';

import 'theme/theme.dart';

final _token = RegExp(
  r'(//.*$)|("(?:[^"\\\n]|\\.)*"|'
  r"'(?:[^'\\\n]|\\.)*')"
  r'|\b(import|from|export|const|let|var|async|await|return|if|else|function|fn|pub|use|mut|impl|struct|enum|match|class|new)\b|\b(\d[\d_.]*)\b',
  multiLine: true,
);

TextSpan highlight(BuildContext context, String code) {
  final ws = context.ws;
  final base = monoStyle(context);
  final spans = <TextSpan>[];
  var start = 0;
  for (final match in _token.allMatches(code)) {
    if (match.start > start) {
      spans.add(TextSpan(text: code.substring(start, match.start)));
    }
    final color = match.group(1) != null
        ? ws.synCom
        : match.group(2) != null
        ? ws.synStr
        : match.group(3) != null
        ? ws.synKw
        : ws.synNum;
    spans.add(
      TextSpan(
        text: match.group(0),
        style: TextStyle(color: color),
      ),
    );
    start = match.end;
  }
  if (start < code.length) spans.add(TextSpan(text: code.substring(start)));
  return TextSpan(style: base, children: spans);
}
