import 'package:flutter/material.dart';

import 'core.dart';
import 'own/editor.dart';
import 'own/model.dart';

void main() => runApp(const MaterialApp(debugShowCheckedModeBanner: false, home: OwnSpikePage()));

SectionCore sampleCore(int paragraphs) {
  final core = SectionCore();
  final ops = <Map<String, Object?>>[];
  var at = 1;
  for (var i = 0; i < paragraphs; i++) {
    if (i > 0) ops.add({'op': 'split', 'at': at++, 'type': 'paragraph'});
    final text = i == 0
        ? 'This is important for everyone, see the docs.'
        : 'Paragraph ${i + 1}: Liquid keeps a Section as one text, so a selection crosses Paragraphs freely.';
    ops.add({'op': 'splice', 'at': at, 'ins': text});
    at += text.length;
  }
  ops.add({'op': 'mark', 'start': 9, 'end': 18, 'name': 'bold'});
  ops.add({'op': 'mark', 'start': 42, 'end': 46, 'name': 'link', 'value': 'https://example.com'});
  core.apply(ops);
  return core;
}

/// Two devices on Liquid's own editor; Sync merges both ways.
class OwnSpikePage extends StatefulWidget {
  const OwnSpikePage({super.key});

  @override
  State<OwnSpikePage> createState() => _OwnSpikePageState();
}

class _OwnSpikePageState extends State<OwnSpikePage> {
  late SectionModel a;
  late SectionModel b;

  @override
  void initState() {
    super.initState();
    _load(3);
  }

  void _load(int paragraphs) {
    final core = sampleCore(paragraphs);
    a = SectionModel(core);
    b = SectionModel(core.fork());
  }

  void _sync() {
    a.core.merge(b.core);
    b.core.merge(a.core);
    a.reload();
    b.reload();
  }

  Widget _device(String name, SectionModel model) => Expanded(
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.all(8),
          child: ValueListenableBuilder(
            valueListenable: model.lastSync,
            builder: (context, micros, _) => Text('$name · last sync ${micros / 1000} ms'),
          ),
        ),
        Expanded(
          child: DecoratedBox(
            decoration: BoxDecoration(border: Border.all(color: Colors.black12)),
            child: SectionEditor(key: ObjectKey(model), model: model),
          ),
        ),
      ],
    ),
  );

  @override
  Widget build(BuildContext context) => Scaffold(
    appBar: AppBar(
      title: const Text('Liquid editor spike'),
      actions: [
        TextButton(onPressed: _sync, child: const Text('Sync')),
        TextButton(onPressed: () => setState(() => _load(1000)), child: const Text('1,000 paragraphs')),
      ],
    ),
    body: Row(children: [_device('Device A', a), _device('Device B', b)]),
  );
}
