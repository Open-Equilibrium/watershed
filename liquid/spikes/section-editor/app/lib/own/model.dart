import 'package:flutter/foundation.dart';

import '../core.dart';

typedef Block = ({String type, int indent, bool done});
typedef Mark = ({String name, Object value, int start, int end});
typedef Pos = ({int para, int offset});

const paragraph = (type: 'paragraph', indent: 0, done: false);
const growing = {'bold', 'italic', 'strike'};

int compare(Pos a, Pos b) => a.para != b.para ? a.para - b.para : a.offset - b.offset;

/// One Paragraph as the editor shows it; [version] tells its view to repaint.
class Para {
  Para(this.marker, this.block, this.text, this.marks);

  String marker;
  Block block;
  String text;
  List<Mark> marks;
  final version = ValueNotifier(0);

  void changed() => version.value++;

  Map<String, Object> marksAt(int offset) => {
    for (final m in marks)
      if (m.start <= offset && offset < m.end) m.name: m.value,
  };
}

/// The Section as Paragraphs, kept equal to its core by sending each edit as
/// the core's own ops: a splice, a marker, a mark. Nothing is diffed.
class SectionModel extends ChangeNotifier {
  SectionModel(this.core) {
    reload();
  }

  final SectionCore core;
  List<Para> paras = [];
  final opLog = <Map<String, Object?>>[];
  final lastSync = ValueNotifier(0);

  int flat(Pos p) {
    var at = 0;
    for (var i = 0; i < p.para; i++) {
      at += 1 + paras[i].text.length;
    }
    return at + 1 + p.offset;
  }

  List<Object?> _send(List<Map<String, Object?>> ops) {
    final watch = Stopwatch()..start();
    opLog.addAll(ops);
    final result = core.apply(ops);
    lastSync.value = watch.elapsedMicroseconds;
    return result;
  }

  void reload() {
    final out = <Para>[];
    for (final span in core.spans()) {
      if (span['block'] case final String marker) {
        out.add(
          Para(
            marker,
            (
              type: span['type'] as String? ?? 'paragraph',
              indent: (span['indent'] as num?)?.toInt() ?? 0,
              done: span['done'] == true,
            ),
            '',
            [],
          ),
        );
      } else {
        final p = out.last;
        final start = p.text.length;
        p.text += span['text'] as String;
        (span['marks'] as Map).forEach(
          (name, value) =>
              p.marks.add((name: name as String, value: value as Object, start: start, end: p.text.length)),
        );
      }
    }
    paras = out;
    notifyListeners();
  }

  /// Inserts [text] with exactly [wanted] marks; the core corrects whatever
  /// Automerge's expand rules gave it, so Dart never predicts them.
  void insert(Pos at, String text, Map<String, Object> wanted) {
    if (text.isEmpty) return;
    final p = paras[at.para];
    final o = at.offset;
    final n = text.length;
    p.text = p.text.replaceRange(o, o, text);
    p.marks = [
      for (final m in p.marks)
        if (m.end <= o)
          m
        else if (m.start >= o)
          (name: m.name, value: m.value, start: m.start + n, end: m.end + n)
        else
          (name: m.name, value: m.value, start: m.start, end: m.end + n),
    ];
    for (final name in {for (final m in p.marks) m.name}) {
      _cut(p, name, o, o + n);
    }
    for (final e in wanted.entries) {
      p.marks.add((name: e.key, value: e.value, start: o, end: o + n));
    }
    _send([
      {'op': 'insert', 'at': flat(at), 'text': text, 'marks': wanted},
    ]);
    p.changed();
  }

  void _cut(Para p, String name, int from, int to) {
    p.marks = [
      for (final m in p.marks)
        if (m.name != name || m.end <= from || m.start >= to)
          m
        else ...[
          if (m.start < from) (name: m.name, value: m.value, start: m.start, end: from),
          if (m.end > to) (name: m.name, value: m.value, start: to, end: m.end),
        ],
    ];
  }

  /// Deletes from [a] to [b]; across Paragraphs it removes their markers, so
  /// the first Paragraph keeps its marker and the text after [b] joins it.
  Pos delete(Pos a, Pos b) {
    if (compare(a, b) > 0) (a, b) = (b, a);
    if (compare(a, b) == 0) return a;
    final from = flat(a);
    final count = flat(b) - from;
    final first = paras[a.para];
    final last = paras[b.para];
    final tail = last.text.substring(b.offset);
    final tailMarks = [
      for (final m in last.marks)
        if (m.end > b.offset)
          (
            name: m.name,
            value: m.value,
            start: (m.start < b.offset ? b.offset : m.start) - b.offset + a.offset,
            end: m.end - b.offset + a.offset,
          ),
    ];
    first.marks = [
      for (final m in first.marks)
        if (m.start < a.offset)
          (name: m.name, value: m.value, start: m.start, end: m.end < a.offset ? m.end : a.offset),
      ...tailMarks,
    ];
    first.text = first.text.substring(0, a.offset) + tail;
    if (b.para > a.para) paras.removeRange(a.para + 1, b.para + 1);
    _send([
      {'op': 'splice', 'at': from, 'del': count},
    ]);
    first.changed();
    if (b.para > a.para) notifyListeners();
    return a;
  }

  /// ⏎: a new marker. At a Paragraph's start the new, empty one goes above, so
  /// the Paragraph keeps its marker.
  Pos split(Pos at) {
    final p = paras[at.para];
    if (at.offset == 0 && p.text.isNotEmpty) {
      final id =
          _send([
                {
                  'op': 'split',
                  'at': flat(at) - 1,
                  ...{'type': 'paragraph', 'indent': 0, 'done': false},
                },
              ]).single
              as String;
      paras.insert(at.para, Para(id, paragraph, '', []));
      notifyListeners();
      return (para: at.para + 1, offset: 0);
    }
    final next = switch (p.block.type) {
      'bullet' || 'numbered' || 'todo' => (type: p.block.type, indent: p.block.indent, done: false),
      _ => paragraph,
    };
    final id =
        _send([
              {'op': 'split', 'at': flat(at), 'type': next.type, 'indent': next.indent, 'done': next.done},
            ]).single
            as String;
    final tail = Para(id, next, p.text.substring(at.offset), [
      for (final m in p.marks)
        if (m.end > at.offset)
          (
            name: m.name,
            value: m.value,
            start: (m.start < at.offset ? at.offset : m.start) - at.offset,
            end: m.end - at.offset,
          ),
    ]);
    p.text = p.text.substring(0, at.offset);
    p.marks = [
      for (final m in p.marks)
        if (m.start < at.offset)
          (name: m.name, value: m.value, start: m.start, end: m.end < at.offset ? m.end : at.offset),
    ];
    paras.insert(at.para + 1, tail);
    p.changed();
    notifyListeners();
    return (para: at.para + 1, offset: 0);
  }

  void setBlock(int index, Block block) {
    final p = paras[index];
    p.block = block;
    _send([
      {'op': 'block', 'id': p.marker, 'type': block.type, 'indent': block.indent, 'done': block.done},
    ]);
    p.changed();
  }

  /// ⌘B and its kind: marks the range unless all of it has the mark.
  void toggle(Pos a, Pos b, String name) {
    if (compare(a, b) > 0) (a, b) = (b, a);
    var all = true;
    for (var i = a.para; i <= b.para; i++) {
      final p = paras[i];
      final from = i == a.para ? a.offset : 0;
      final to = i == b.para ? b.offset : p.text.length;
      for (var o = from; o < to; o++) {
        if (!p.marksAt(o).containsKey(name)) all = false;
      }
    }
    final ops = <Map<String, Object?>>[];
    for (var i = a.para; i <= b.para; i++) {
      final p = paras[i];
      final from = i == a.para ? a.offset : 0;
      final to = i == b.para ? b.offset : p.text.length;
      if (from >= to) continue;
      _cut(p, name, from, to);
      if (!all) p.marks.add((name: name, value: true, start: from, end: to));
      final start = flat((para: i, offset: from));
      ops.add(
        all
            ? {'op': 'unmark', 'start': start, 'end': start + to - from, 'name': name}
            : {'op': 'mark', 'start': start, 'end': start + to - from, 'name': name, 'value': true},
      );
      p.changed();
    }
    _send(ops);
  }

  /// The model in the core's shape, to compare with the core's spans.
  List<Object> shape() {
    final out = <Object>[];
    for (final p in paras) {
      out.add({'block': p.marker, 'type': p.block.type, 'indent': p.block.indent, 'done': p.block.done});
      for (var o = 0; o < p.text.length; o++) {
        final marks = p.marksAt(o);
        final last = out.last;
        if (last is Map && last.containsKey('text') && mapEquals(last['marks'] as Map, marks)) {
          out[out.length - 1] = {'text': '${last['text']}${p.text[o]}', 'marks': marks};
        } else {
          out.add({'text': p.text[o], 'marks': marks});
        }
      }
    }
    return out;
  }
}
