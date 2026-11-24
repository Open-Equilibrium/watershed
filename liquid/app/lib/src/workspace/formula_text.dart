import 'package:flutter/material.dart';

import '../theme/theme.dart';
import '../theme/tokens.g.dart';

const chipMark = '￼';

const formulaFunctions = [
  'SUM',
  'AVERAGE',
  'MIN',
  'MAX',
  'COUNT',
  'COUNTA',
  'COUNTBLANK',
  'SUMIF',
  'COUNTIF',
  'AVERAGEIF',
  'PRODUCT',
  'MEDIAN',
  'IF',
  'IFERROR',
  'AND',
  'OR',
  'NOT',
  'ROUND',
  'ROUNDUP',
  'ROUNDDOWN',
  'ABS',
  'INT',
  'MOD',
  'POWER',
  'SQRT',
  'LEN',
  'UPPER',
  'LOWER',
  'TRIM',
  'LEFT',
  'RIGHT',
  'MID',
  'CONCAT',
  'INDEX',
  'ISBLANK',
  'ISNUMBER',
  'ISTEXT',
  'ISERROR',
];

class Reference {
  const Reference(this.token, this.name);

  final String token;
  final String name;
}

class Suggestion {
  const Suggestion(this.name, this.detail, {this.reference});

  final String name;
  final String detail;
  final Reference? reference;
}

final _token = RegExp(r'"(?:[^"]|"")*"|\{[0-9a-f.]+\}');

String _tokens(String formula, String Function(String token, int at) chip) {
  var at = 0;
  return formula.replaceAllMapped(
    _token,
    (match) => match[0]!.startsWith('"') ? match[0]! : chip(match[0]!, at++),
  );
}

final _number = RegExp(r'^-?\d+(\.\d+)?([eE][+-]?\d+)?$');
final _stop = RegExp(r'[(),+\-−*/^&=<>%￼]');

String? _literal(String source) {
  if (source.length < 2 || !source.startsWith('"') || !source.endsWith('"')) {
    return null;
  }
  final inner = source.substring(1, source.length - 1);
  if (inner.replaceAll('""', '').contains('"')) return null;
  return inner.replaceAll('""', '"');
}

bool isComputed(String formula) {
  final source = formula.trim();
  return source.isNotEmpty &&
      !_number.hasMatch(source) &&
      _literal(source) == null;
}

(String, List<Reference>) editable(
  String formula,
  List<String?> names, {
  bool always = false,
}) {
  final source = formula.trim();
  if (always && source.isEmpty) return ('', const []);
  if (!always && !isComputed(source)) {
    return (_literal(source) ?? source, const []);
  }
  final chips = <Reference>[];
  final text = _tokens(source, (token, at) {
    chips.add(
      Reference(token, at < names.length ? names[at] ?? '#REF!' : '#REF!'),
    );
    return chipMark;
  });
  return ('=$text', chips);
}

String storedFormula(
  String text,
  List<Reference> chips, {
  bool always = false,
}) {
  final typed = text.trim();
  if (typed.isEmpty) return '';
  if (always || typed.startsWith('=')) {
    var at = 0;
    return (typed.startsWith('=') ? typed.substring(1) : typed)
        .trim()
        .replaceAllMapped(
          chipMark,
          (_) => at < chips.length ? chips[at++].token : '',
        );
  }
  if (_number.hasMatch(typed)) return typed;
  return '"${typed.replaceAll('"', '""')}"';
}

List<String?> tokenNames(
  String formula,
  String? Function(List<String> ids) name,
) {
  final names = <String?>[];
  _tokens(formula, (token, _) {
    names.add(name(token.substring(1, token.length - 1).split('.')));
    return token;
  });
  return names;
}

String namedFormula(String formula, List<String?> names) => _tokens(
  formula,
  (_, at) => (at < names.length ? names[at] : null) ?? '#REF!',
);

class FormulaText extends TextEditingController {
  bool always = false;
  List<Reference> chips = const [];
  List<Reference> _inserting = const [];

  void load(String text, List<Reference> references) {
    chips = const [];
    super.value = TextEditingValue(
      text: text,
      selection: TextSelection.collapsed(offset: text.length),
    );
    chips = [...references];
  }

  @override
  set value(TextEditingValue next) {
    final before = super.value.text;
    if (before != next.text) {
      chips = _kept(before, next.text, next.selection.baseOffset);
    }
    super.value = next;
  }

  List<Reference> _kept(String before, String after, int caret) {
    var start = 0;
    while (start < before.length &&
        start < after.length &&
        before[start] == after[start]) {
      start++;
    }
    var end = 0;
    while (end < before.length - start &&
        end < after.length - start &&
        before[before.length - 1 - end] == after[after.length - 1 - end]) {
      end++;
    }
    final cut = before.length - after.length;
    if (cut > 0 &&
        caret >= 0 &&
        caret + cut <= before.length &&
        after.startsWith(before.substring(0, caret)) &&
        after.endsWith(before.substring(caret + cut))) {
      start = caret;
      end = before.length - caret - cut;
    }
    int marks(String text) => chipMark.allMatches(text).length;
    final kept = marks(before.substring(0, start));
    final removed = marks(before.substring(start, before.length - end));
    final added = marks(after.substring(start, after.length - end));
    final inserted = [
      for (var at = 0; at < added; at++)
        at < _inserting.length ? _inserting[at] : const Reference('', '?'),
    ];
    return [...chips.take(kept), ...inserted, ...chips.skip(kept + removed)];
  }

  String? get word {
    final text = this.text;
    final caret = selection.baseOffset;
    final floor = text.startsWith('=') ? 1 : 0;
    if ((floor == 0 && !always) || caret < floor || caret > text.length) {
      return null;
    }
    var start = caret;
    while (start > floor && !_stop.hasMatch(text[start - 1])) {
      start--;
    }
    final word = text.substring(start, caret).trimLeft();
    if (word.isEmpty || _number.hasMatch(word)) return null;
    return word;
  }

  void take(Suggestion suggestion) {
    final word = this.word;
    if (word == null) return;
    final caret = selection.baseOffset;
    final start = caret - word.length;
    final reference = suggestion.reference;
    final inserted = reference != null
        ? chipMark
        : suggestion.name == 'ABOVE'
        ? 'ABOVE'
        : '${suggestion.name}(';
    _inserting = [?reference];
    value = TextEditingValue(
      text: text.replaceRange(start, caret, inserted),
      selection: TextSelection.collapsed(offset: start + inserted.length),
    );
    _inserting = const [];
  }

  @override
  TextSpan buildTextSpan({
    required BuildContext context,
    TextStyle? style,
    required bool withComposing,
  }) {
    final parts = text.split(chipMark);
    final children = <InlineSpan>[];
    for (final (at, part) in parts.indexed) {
      if (at > 0) {
        final name = at - 1 < chips.length ? chips[at - 1].name : '?';
        children.add(
          WidgetSpan(
            alignment: PlaceholderAlignment.middle,
            child: FormulaChip(name),
          ),
        );
      }
      if (part.isNotEmpty) children.add(TextSpan(text: part));
    }
    return TextSpan(style: style, children: children);
  }
}

class FormulaChip extends StatelessWidget {
  const FormulaChip(this.name, {super.key});

  final String name;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      margin: const EdgeInsets.symmetric(horizontal: 2),
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space2),
      decoration: BoxDecoration(
        color: ws.accentSubtle,
        borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
      ),
      child: Text(
        name,
        style: TextStyle(
          fontSize: WsMetrics.fsSmall,
          fontWeight: WsMetrics.fwMedium,
          color: ws.accent,
          fontFamily: WsMetrics.fontUi.first,
        ),
      ),
    );
  }
}

List<Suggestion> suggestionsFor(String word, List<Suggestion> known) {
  final typed = word.toLowerCase();
  final starting = <Suggestion>[];
  final containing = <Suggestion>[];
  for (final suggestion in known) {
    final name = suggestion.name.toLowerCase();
    if (name.startsWith(typed)) {
      starting.add(suggestion);
    } else if (name.contains(typed)) {
      containing.add(suggestion);
    }
  }
  return [...starting, ...containing].take(6).toList();
}
