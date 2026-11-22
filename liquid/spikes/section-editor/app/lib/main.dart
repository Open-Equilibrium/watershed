import 'package:flutter/material.dart';
import 'package:super_editor/super_editor.dart';

import 'binding.dart';
import 'core.dart';

void main() => runApp(const SpikeApp());

class SpikeApp extends StatelessWidget {
  const SpikeApp({super.key});

  @override
  Widget build(BuildContext context) =>
      MaterialApp(debugShowCheckedModeBanner: false, title: 'Section editor spike', home: const SpikePage());
}

/// Two devices editing one Section; Sync merges both ways, as the Sync Server would.
class SpikePage extends StatefulWidget {
  const SpikePage({super.key, this.paragraphs = 3});

  final int paragraphs;

  @override
  State<SpikePage> createState() => SpikePageState();
}

class SpikePageState extends State<SpikePage> {
  late SectionBinding a;
  late SectionBinding b;

  @override
  void initState() {
    super.initState();
    _load(widget.paragraphs);
  }

  void _load(int paragraphs) {
    final core = SectionCore();
    final ops = <Map<String, Object?>>[];
    var at = 1;
    for (var i = 0; i < paragraphs; i++) {
      if (i > 0) {
        ops.add({'op': 'split', 'at': at, 'type': 'paragraph'});
        at++;
      }
      final text = i == 0
          ? 'Hello world, this is the first paragraph.'
          : 'Paragraph ${i + 1}: Liquid keeps a Section as one text, so selection crosses paragraphs.';
      ops.add({'op': 'splice', 'at': at, 'ins': text});
      at += text.length;
    }
    core.apply(ops);
    a = SectionBinding(core);
    b = SectionBinding(core.fork());
  }

  void sync() {
    a.core.merge(b.core);
    b.core.merge(a.core);
    a.reloadFromCore();
    b.reloadFromCore();
    setState(() {});
  }

  Widget _device(String name, SectionBinding binding) => Expanded(
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.all(8),
          child: ValueListenableBuilder(
            valueListenable: binding.lastSync,
            builder: (context, micros, _) => Text('$name · last sync ${micros / 1000} ms'),
          ),
        ),
        Expanded(
          child: DecoratedBox(
            decoration: BoxDecoration(border: Border.all(color: Colors.black12)),
            child: SuperEditor(key: ObjectKey(binding), editor: binding.editor, stylesheet: defaultStylesheet),
          ),
        ),
      ],
    ),
  );

  @override
  Widget build(BuildContext context) => Scaffold(
    appBar: AppBar(
      title: const Text('Section editor spike'),
      actions: [
        TextButton(onPressed: sync, child: const Text('Sync')),
        TextButton(onPressed: () => setState(() => _load(1000)), child: const Text('1,000 paragraphs')),
      ],
    ),
    body: Row(children: [_device('Device A', a), _device('Device B', b)]),
  );
}
