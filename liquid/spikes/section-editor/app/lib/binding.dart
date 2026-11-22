import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:super_editor/super_editor.dart';

import 'core.dart';

typedef Block = ({String type, int indent, bool done});

const markNames = ['bold', 'italic', 'strike', 'code', 'link'];

Attribution? attributionFor(String name, Object? value) => switch (name) {
  'bold' => boldAttribution,
  'italic' => italicsAttribution,
  'strike' => strikethroughAttribution,
  'code' => codeAttribution,
  'link' when value is String => LinkAttribution(value),
  _ => null,
};

(String, Object)? markOf(Attribution a) {
  if (a == boldAttribution) return ('bold', true);
  if (a == italicsAttribution) return ('italic', true);
  if (a == strikethroughAttribution) return ('strike', true);
  if (a == codeAttribution) return ('code', true);
  if (a is LinkAttribution) return ('link', a.plainTextUri);
  return null;
}

Map<String, Object> marksAt(AttributedText text, int offset) => {
  for (final a in text.getAllAttributionsAt(offset))
    if (markOf(a) case (final name, final value)) name: value,
};

Block blockOf(DocumentNode node) => switch (node) {
  ListItemNode(:final type, :final indent) => (
    type: type == ListItemType.ordered ? 'numbered' : 'bullet',
    indent: indent,
    done: false,
  ),
  TaskNode(:final indent, :final isComplete) => (type: 'todo', indent: indent, done: isComplete),
  ParagraphNode() => (
    type: switch (node.getMetadataValue('blockType')) {
      header1Attribution => 'heading1',
      header2Attribution => 'heading2',
      header3Attribution => 'heading3',
      blockquoteAttribution => 'quote',
      _ => 'paragraph',
    },
    indent: 0,
    done: false,
  ),
  _ => throw UnsupportedError('${node.runtimeType}'),
};

DocumentNode nodeOf(String id, Block block, AttributedText text) => switch (block.type) {
  'bullet' => ListItemNode.unordered(id: id, text: text, indent: block.indent),
  'numbered' => ListItemNode.ordered(id: id, text: text, indent: block.indent),
  'todo' => TaskNode(id: id, text: text, isComplete: block.done, indent: block.indent),
  final type => ParagraphNode(
    id: id,
    text: text,
    metadata: {
      'blockType': switch (type) {
        'heading1' => header1Attribution,
        'heading2' => header2Attribution,
        'heading3' => header3Attribution,
        'quote' => blockquoteAttribution,
        _ => paragraphAttribution,
      },
    },
  ),
};

AttributedText textOf(DocumentNode node) => (node as TextNode).text;

/// Builds editor nodes from the core's spans; a node's id is its marker's id.
List<DocumentNode> nodesFromSpans(List<Map<String, Object?>> spans) {
  final nodes = <DocumentNode>[];
  String? id;
  Block? block;
  var text = StringBuffer();
  final marks = <(String, Object, int, int)>[];
  void flush() {
    final nodeId = id;
    if (nodeId == null) return;
    final attributed = AttributedText(text.toString());
    for (final (name, value, start, end) in marks) {
      attributed.addAttribution(attributionFor(name, value)!, SpanRange(start, end - 1), autoMerge: true);
    }
    nodes.add(nodeOf(nodeId, block!, attributed));
  }

  for (final span in spans) {
    if (span['block'] case final String marker) {
      flush();
      id = marker;
      block = (
        type: span['type'] as String? ?? 'paragraph',
        indent: (span['indent'] as num?)?.toInt() ?? 0,
        done: span['done'] == true,
      );
      text = StringBuffer();
      marks.clear();
    } else {
      final run = span['text'] as String;
      final start = text.length;
      text.write(run);
      (span['marks'] as Map).forEach((name, value) => marks.add((name as String, value as Object, start, text.length)));
    }
  }
  flush();
  return nodes;
}

final class _Marker {
  const _Marker(this.node);
  final String node;
  @override
  bool operator ==(Object other) => other is _Marker && other.node == node;
  @override
  int get hashCode => node.hashCode;
}

class _Cached {
  _Cached(DocumentNode node) : id = node.id, block = blockOf(node), chars = textOf(node).toPlainText().codeUnits;
  final String id;
  final Block block;
  final List<int> chars;
  int get length => 1 + chars.length;
}

/// Keeps one Section's core in step with a super_editor document.
///
/// Paragraphs map to block markers: ⏎ inserts one and ⌫ at a start deletes
/// one, so no text moves between objects. Text edits become splices, marks
/// become Automerge marks with Peritext's expand rules.
class SectionBinding {
  SectionBinding(this.core) {
    final nodes = nodesFromSpans(core.spans());
    document = MutableDocument(nodes: nodes);
    composer = MutableDocumentComposer();
    editor = createDefaultDocumentEditor(document: document, composer: composer);
    for (final node in nodes) {
      _marker[node.id] = node.id;
    }
    _cache = [for (final node in document) _Cached(node)];
    editor.addListener(FunctionalEditListener(_onEdit));
    composer.composingRegion.addListener(_onComposing);
  }

  final SectionCore core;
  late final MutableDocument document;
  late final MutableDocumentComposer composer;
  late final Editor editor;

  final _marker = <String, String>{};
  late List<_Cached> _cache;
  final _touched = <String>{};
  final _attributionEvents = <AttributionChangeEvent>[];
  final _inserted = <(String, int)>{};
  final _deleted = <(String, int)>{};
  bool _remote = false;

  /// Every op sent to the core, for diagnostics; whoever reads it clears it.
  final opLog = <Map<String, Object?>>[];

  /// Microseconds the last sync took, from the editor's events to the core's answer.
  final lastSync = ValueNotifier<int>(0);

  String markerOf(String nodeId) => _marker[nodeId]!;

  void _onEdit(List<EditEvent> events) {
    if (_remote) return;
    for (final event in events) {
      if (event is DocumentEdit && event.change is NodeDocumentChange) {
        final change = event.change as NodeDocumentChange;
        _touched.add(change.nodeId);
        if (change is AttributionChangeEvent) _attributionEvents.add(change);
        if (change is TextInsertionEvent) _inserted.add((change.nodeId, change.offset));
        if (change is TextDeletedEvent) _deleted.add((change.nodeId, change.offset));
      }
    }
    if (_touched.isEmpty || _scheduled) return;
    _scheduled = true;
    scheduleMicrotask(() {
      _scheduled = false;
      if (composer.composingRegion.value == null && _touched.isNotEmpty) _sync();
    });
  }

  bool _scheduled = false;

  void _onComposing() {
    if (composer.composingRegion.value == null && _touched.isNotEmpty && !_remote) _sync();
  }

  void _sync() {
    final watch = Stopwatch()..start();
    final fresh = document.toList();
    final old = _cache;
    var head = 0;
    while (head < old.length &&
        head < fresh.length &&
        old[head].id == fresh[head].id &&
        !_touched.contains(old[head].id)) {
      head++;
    }
    var tail = 0;
    while (tail < old.length - head &&
        tail < fresh.length - head &&
        old[old.length - 1 - tail].id == fresh[fresh.length - 1 - tail].id &&
        !_touched.contains(old[old.length - 1 - tail].id)) {
      tail++;
    }
    var base = 0;
    for (var i = 0; i < head; i++) {
      base += old[i].length;
    }
    final oldWindow = old.sublist(head, old.length - tail);
    final newWindow = fresh.sublist(head, fresh.length - tail);
    final ops = <Map<String, Object?>>[];
    final created = <String>[];
    _diff(oldWindow, newWindow, base, ops, created);
    final known = {for (final c in oldWindow) c.id: c.block};
    for (final node in newWindow) {
      final block = blockOf(node);
      final before = known[node.id];
      if (before != null && before != block) {
        ops.add({'op': 'block', 'id': _marker[node.id], ...?_blockJson(block)});
      }
    }
    _attributionOps(fresh, ops);
    opLog.addAll(ops);
    final results = core.apply(ops);
    var next = 0;
    for (var i = 0; i < ops.length; i++) {
      if (ops[i]['op'] == 'split') _marker[created[next++]] = results[i] as String;
    }
    for (final c in oldWindow) {
      if (!newWindow.any((n) => n.id == c.id)) _marker.remove(c.id);
    }
    _cache = [...old.sublist(0, head), for (final n in newWindow) _Cached(n), ...old.sublist(old.length - tail)];
    _touched.clear();
    _attributionEvents.clear();
    _inserted.clear();
    _deleted.clear();
    lastSync.value = watch.elapsedMicroseconds;
  }

  Map<String, Object?>? _blockJson(Block block) => {'type': block.type, 'indent': block.indent, 'done': block.done};

  List<Object> _tokensOf(List<String> ids, List<List<int>> chars) => [
    for (var i = 0; i < ids.length; i++) ...[_Marker(ids[i]), ...chars[i]],
  ];

  void _diff(
    List<_Cached> oldWindow,
    List<DocumentNode> newWindow,
    int base,
    List<Map<String, Object?>> ops,
    List<String> created,
  ) {
    final oldTokens = _tokensOf([for (final c in oldWindow) c.id], [for (final c in oldWindow) c.chars]);
    final newTokens = _tokensOf(
      [for (final n in newWindow) n.id],
      [for (final n in newWindow) textOf(n).toPlainText().codeUnits],
    );
    final oldOwner = <(String, int)>[];
    for (final c in oldWindow) {
      oldOwner.add((c.id, -1));
      for (var o = 0; o < c.chars.length; o++) {
        oldOwner.add((c.id, o));
      }
    }
    final owner = <(DocumentNode, int)>[];
    for (final node in newWindow) {
      owner.add((node, -1));
      for (var o = 0; o < textOf(node).length; o++) {
        owner.add((node, o));
      }
    }
    final oldAt = {
      for (var i = 0; i < oldTokens.length; i++)
        if (oldTokens[i] is _Marker) oldTokens[i]: i,
    };
    final anchors = <(int, int)>[];
    var lastOld = -1;
    for (var j = 0; j < newTokens.length; j++) {
      final i = oldAt[newTokens[j]];
      if (i != null && i > lastOld) {
        anchors.add((i, j));
        lastOld = i;
      }
    }
    var shift = 0;
    var oldStart = 0;
    var newStart = 0;
    for (final (oi, nj) in [...anchors, (oldTokens.length, newTokens.length)]) {
      shift = _segment(oldTokens, oldStart, oi, newTokens, newStart, nj, oldOwner, owner, base, shift, ops, created);
      oldStart = oi + 1;
      newStart = nj + 1;
    }
  }

  int _segment(
    List<Object> oldTokens,
    int os,
    int oe,
    List<Object> newTokens,
    int ns,
    int ne,
    List<(String, int)> oldOwner,
    List<(DocumentNode, int)> owner,
    int base,
    int shift,
    List<Map<String, Object?>> ops,
    List<String> created,
  ) {
    final oldSegmentStart = os;
    final newSegmentStart = ns;
    while (os < oe && ns < ne && oldTokens[os] == newTokens[ns]) {
      os++;
      ns++;
    }
    while (oe > os && ne > ns && oldTokens[oe - 1] == newTokens[ne - 1]) {
      oe--;
      ne--;
    }
    // Among equally valid places for a pure deletion or insertion in a run of
    // equal characters, take the one the editor reported.
    if (ne == ns && oe > os) {
      var slide = 0;
      while (os - slide > oldSegmentStart && oldTokens[os - slide - 1] == oldTokens[oe - slide - 1]) {
        final (id, offset) = oldOwner[os - slide];
        if (_deleted.contains((id, offset))) break;
        slide++;
      }
      final (id, offset) = oldOwner[os - slide];
      if (_deleted.contains((id, offset))) {
        os -= slide;
        oe -= slide;
        ns -= slide;
        ne -= slide;
      }
    } else if (oe == os && ne > ns) {
      var slide = 0;
      while (ns - slide > newSegmentStart && newTokens[ns - slide - 1] == newTokens[ne - slide - 1]) {
        final (node, offset) = owner[ns - slide];
        if (_inserted.contains((node.id, offset))) break;
        slide++;
      }
      final (node, offset) = owner[ns - slide];
      if (_inserted.contains((node.id, offset))) {
        os -= slide;
        oe -= slide;
        ns -= slide;
        ne -= slide;
      }
    }
    var at = base + os + shift;
    if (oe > os) {
      ops.add({'op': 'splice', 'at': at, 'del': oe - os});
      shift -= oe - os;
    }
    var run = StringBuffer();
    var runStart = ns;
    void flush(int end) {
      if (run.isEmpty) return;
      ops.add({'op': 'splice', 'at': at, 'ins': run.toString()});
      _runMarks(owner, runStart, end, at, ops);
      at += run.length;
      shift += run.length;
      run = StringBuffer();
    }

    for (var j = ns; j < ne; j++) {
      final token = newTokens[j];
      if (token is _Marker) {
        flush(j);
        final node = owner[j].$1;
        ops.add({'op': 'split', 'at': at, ...?_blockJson(blockOf(node))});
        created.add(node.id);
        at += 1;
        shift += 1;
        runStart = j + 1;
      } else {
        if (run.isEmpty) runStart = j;
        run.writeCharCode(token as int);
      }
    }
    flush(ne);
    return shift;
  }

  /// Gives inserted text exactly the editor's marks, whatever Automerge's
  /// expand rules would have given it.
  void _runMarks(List<(DocumentNode, int)> owner, int from, int to, int at, List<Map<String, Object?>> ops) {
    final node = owner[from].$1;
    final text = textOf(node);
    final first = owner[from].$2;
    final last = owner[to - 1].$2;
    final neighbours = <String>{
      // At a Paragraph start, Automerge may carry a mark over the marker.
      if (first == 0) ...['bold', 'italic', 'strike'],
      if (first > 0) ...marksAt(text, first - 1).keys,
      if (last + 1 < text.length) ...marksAt(text, last + 1).keys,
    };
    for (final name in markNames) {
      var o = first;
      while (o <= last) {
        final value = marksAt(text, o)[name];
        var end = o + 1;
        while (end <= last && marksAt(text, end)[name] == value) {
          end++;
        }
        final start = at + o - first;
        final stop = at + end - first;
        if (value != null) {
          ops.add({'op': 'mark', 'start': start, 'end': stop, 'name': name, 'value': value});
        } else if (neighbours.contains(name)) {
          ops.add({'op': 'unmark', 'start': start, 'end': stop, 'name': name});
        }
        o = end;
      }
    }
  }

  void _attributionOps(List<DocumentNode> fresh, List<Map<String, Object?>> ops) {
    if (_attributionEvents.isEmpty) return;
    final starts = <String, int>{};
    var position = 0;
    for (final node in fresh) {
      starts[node.id] = position;
      position += 1 + textOf(node).length;
    }
    for (final event in _attributionEvents) {
      final start = starts[event.nodeId];
      if (start == null) continue;
      for (final attribution in event.attributions) {
        final mark = markOf(attribution);
        if (mark == null) continue;
        final from = start + 1 + event.range.start;
        final to = start + 1 + event.range.end + 1;
        ops.add(
          event.change == AttributionChange.added
              ? {'op': 'mark', 'start': from, 'end': to, 'name': mark.$1, 'value': mark.$2}
              : {'op': 'unmark', 'start': from, 'end': to, 'name': mark.$1},
        );
      }
    }
  }

  /// Shows the core's current state after a merge: changed and new
  /// Paragraphs are replaced or inserted, gone ones deleted.
  void reloadFromCore() {
    final nodes = nodesFromSpans(core.spans());
    final byMarker = {for (final e in _marker.entries) e.value: e.key};
    final requests = <EditRequest>[];
    final wanted = <String>{};
    final placed = <DocumentNode>[];
    for (final node in nodes) {
      final id = byMarker[node.id] ?? node.id;
      wanted.add(id);
      placed.add(nodeOf(id, blockOf(node), textOf(node)));
    }
    for (final node in document.toList()) {
      if (!wanted.contains(node.id)) requests.add(DeleteNodeRequest(nodeId: node.id));
    }
    for (var i = 0; i < placed.length; i++) {
      final node = placed[i];
      final existing = document.getNodeById(node.id);
      if (existing == null) {
        requests.add(InsertNodeAtIndexRequest(nodeIndex: i, newNode: node));
      } else if (!existing.hasEquivalentContent(node)) {
        requests.add(ReplaceNodeRequest(existingNodeId: node.id, newNode: node));
      }
    }
    for (final node in nodes) {
      _marker[byMarker[node.id] ?? node.id] = node.id;
    }
    _remote = true;
    try {
      if (requests.isNotEmpty) editor.execute(requests);
    } finally {
      _remote = false;
    }
    _cache = [for (final node in document) _Cached(node)];
  }

  /// The editor's document in the core's shape, to compare with [SectionCore.spans].
  List<Object> editorShape() => [
    for (final node in document) ...[
      {'block': _marker[node.id], ...?_blockJson(blockOf(node))},
      ..._runsOf(textOf(node)),
    ],
  ];

  static List<Object> coreShape(List<Map<String, Object?>> spans) {
    final out = <Object>[];
    for (final span in spans) {
      if (span['block'] != null) {
        out.add({
          'block': span['block'],
          'type': span['type'] ?? 'paragraph',
          'indent': (span['indent'] as num?)?.toInt() ?? 0,
          'done': span['done'] == true,
        });
      } else {
        out.add({'text': span['text'], 'marks': Map<String, Object?>.from(span['marks'] as Map)});
      }
    }
    return _mergeRuns(out);
  }

  static List<Object> _runsOf(AttributedText text) {
    final out = <Object>[];
    for (var o = 0; o < text.length; o++) {
      out.add({'text': text.toPlainText()[o], 'marks': marksAt(text, o)});
    }
    return _mergeRuns(out);
  }

  static List<Object> _mergeRuns(List<Object> items) {
    final out = <Object>[];
    for (final item in items) {
      if (item is Map && item.containsKey('text') && (item['text'] as String).isEmpty) continue;
      final last = out.isEmpty ? null : out.last;
      if (item is Map &&
          last is Map &&
          item.containsKey('text') &&
          last.containsKey('text') &&
          mapEquals(item['marks'] as Map, last['marks'] as Map)) {
        out[out.length - 1] = {'text': '${last['text']}${item['text']}', 'marks': last['marks']};
      } else {
        out.add(item);
      }
    }
    return out;
  }
}
