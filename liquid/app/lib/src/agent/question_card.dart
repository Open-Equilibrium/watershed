import 'package:flutter/material.dart';

import '../controls.dart';
import '../rust/api/events.dart';
import '../session/session_state.dart';
import '../shortcuts/keymap.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';

/// A selection that narrows as the user types; arrows move and Enter picks (approved mockup).
class _OptionList extends StatefulWidget {
  const _OptionList({
    required this.options,
    required this.onPick,
    required this.focusNode,
  });

  final List<String> options;
  final ValueChanged<String> onPick;
  final FocusNode focusNode;

  @override
  State<_OptionList> createState() => _OptionListState();
}

class _OptionListState extends State<_OptionList> {
  static const _rowHeight = 22.0;
  final _filter = TextEditingController();
  final _scroll = ScrollController();
  int _highlight = 0;

  @override
  void dispose() {
    _filter.dispose();
    _scroll.dispose();
    super.dispose();
  }

  List<String> get _shown {
    final query = _filter.text.toLowerCase();
    return [
      for (final option in widget.options)
        if (option.toLowerCase().contains(query)) option,
    ];
  }

  void _move(int by) {
    final count = _shown.length;
    if (count == 0) return;
    setState(() => _highlight = (_highlight + by).clamp(0, count - 1));
    if (!_scroll.hasClients) return;
    // Keeps the highlighted option in view.
    final top = _highlight * _rowHeight;
    final position = _scroll.position;
    if (top < position.pixels) {
      _scroll.jumpTo(top);
    } else if (top + _rowHeight >
        position.pixels + position.viewportDimension) {
      _scroll.jumpTo(top + _rowHeight - position.viewportDimension);
    }
  }

  void _pick() {
    final shown = _shown;
    if (_highlight < shown.length) widget.onPick(shown[_highlight]);
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final shown = _shown;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Row(
          children: [
            Expanded(
              child: Container(
                padding: const EdgeInsets.symmetric(
                  horizontal: WsMetrics.space2,
                  vertical: WsMetrics.space1,
                ),
                decoration: BoxDecoration(
                  border: Border.all(color: ws.borderStrong),
                  borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                ),
                child: CallbackShortcuts(
                  bindings: {
                    ...Fixed.moveThroughOptions.bind([
                      () => _move(-1),
                      () => _move(1),
                    ]),
                    for (final keys in Keymap.of(
                      context,
                    ).bindings(Command.answer))
                      keys.activator: _pick,
                  },
                  child: TextField(
                    key: const Key('filter'),
                    controller: _filter,
                    focusNode: widget.focusNode,
                    style: const TextStyle(fontSize: WsMetrics.fsSmall),
                    decoration: const InputDecoration.collapsed(
                      hintText: 'Type to filter',
                    ),
                    onChanged: (_) => setState(() => _highlight = 0),
                  ),
                ),
              ),
            ),
            const SizedBox(width: WsMetrics.space3),
            Text(
              '${shown.length} / ${widget.options.length}',
              style: TextStyle(
                color: ws.textFaint,
                fontSize: WsMetrics.fsSmall,
              ),
            ),
          ],
        ),
        const SizedBox(height: WsMetrics.space1),
        Expanded(
          child: ListView.builder(
            controller: _scroll,
            itemExtent: _rowHeight,
            itemCount: shown.length,
            itemBuilder: (context, index) => InkWell(
              canRequestFocus: false,
              onTap: () => widget.onPick(shown[index]),
              borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
              child: Container(
                alignment: Alignment.centerLeft,
                padding: const EdgeInsets.symmetric(
                  horizontal: WsMetrics.space2,
                ),
                decoration: BoxDecoration(
                  color: index == _highlight ? ws.selected : null,
                  borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                ),
                child: Text(
                  shown[index],
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
            ),
          ),
        ),
      ],
    );
  }
}

/// A question from an agent extension, answered in place of the composer.
class QuestionCard extends StatefulWidget {
  const QuestionCard({super.key, required this.request, required this.state});

  final BridgeRequest request;
  final SessionState state;

  @override
  State<QuestionCard> createState() => _QuestionCardState();
}

class _QuestionCardState extends State<QuestionCard> {
  late final TextEditingController _text = TextEditingController(
    text: widget.request.question!.kind == BridgeQuestionKind.editor
        ? widget.request.question!.detail
        : '',
  );
  final _card = FocusNode(debugLabel: 'question');
  final _field = FocusNode(debugLabel: 'answer');

  @override
  void initState() {
    super.initState();
    // A confirmation takes the keys itself, other questions in their field.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      widget.request.question!.kind == BridgeQuestionKind.confirm
          ? _card.requestFocus()
          : _field.requestFocus();
    });
  }

  @override
  void dispose() {
    _text.dispose();
    _card.dispose();
    _field.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final question = widget.request.question!;
    final state = widget.state;
    final id = widget.request.id;
    final keys = Keymap.of(context);
    final answer = switch (question.kind) {
      BridgeQuestionKind.select => _OptionList(
        options: question.options,
        onPick: (option) => state.answerText(id, option),
        focusNode: _field,
      ),
      BridgeQuestionKind.confirm => Row(
        children: [
          PrimaryButton(
            label: 'Yes',
            shortcut: keys.label(Command.answer),
            onPressed: () => state.answerConfirm(id, true),
          ),
          const SizedBox(width: WsMetrics.space3),
          SecondaryButton(
            label: 'No',
            onPressed: () => state.answerConfirm(id, false),
          ),
        ],
      ),
      // An editor fills the card: Enter adds a line, ⌘Enter answers (approved mockup).
      BridgeQuestionKind.editor => Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Expanded(
            child: CallbackShortcuts(
              bindings: keys.bind({
                Command.answerEditor: () => state.answerText(id, _text.text),
              }),
              child: Container(
                padding: const EdgeInsets.symmetric(
                  horizontal: WsMetrics.space4,
                  vertical: WsMetrics.space2,
                ),
                decoration: BoxDecoration(
                  color: context.ws.code,
                  borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                ),
                child: TextField(
                  key: const Key('answer'),
                  controller: _text,
                  focusNode: _field,
                  maxLines: null,
                  expands: true,
                  keyboardType: TextInputType.multiline,
                  textAlignVertical: TextAlignVertical.top,
                  style: monoStyle(context),
                  decoration: const InputDecoration.collapsed(hintText: ''),
                ),
              ),
            ),
          ),
          const SizedBox(height: WsMetrics.space2),
          Row(
            children: [
              const Hint('⏎ new line'),
              const Spacer(),
              PrimaryButton(
                label: 'Answer',
                shortcut: keys.label(Command.answerEditor),
                onPressed: () => state.answerText(id, _text.text),
              ),
            ],
          ),
        ],
      ),
      BridgeQuestionKind.input => Row(
        children: [
          Expanded(
            child: CallbackShortcuts(
              bindings: keys.bind({
                Command.answer: () => state.answerText(id, _text.text),
              }),
              child: TextField(
                key: const Key('answer'),
                controller: _text,
                focusNode: _field,
                decoration: InputDecoration.collapsed(
                  hintText: question.detail,
                ),
                // The answer keys answer; the field keeps the focus for them.
                onEditingComplete: () {},
              ),
            ),
          ),
          PrimaryButton(
            label: 'Answer',
            shortcut: keys.label(Command.answer),
            onPressed: () => state.answerText(id, _text.text),
          ),
        ],
      ),
    };
    final header = Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Expanded(
          child: SingleChildScrollView(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Text(
                  question.title,
                  style: Theme.of(context).textTheme.labelMedium,
                ),
                if (question.kind == BridgeQuestionKind.confirm &&
                    question.detail.isNotEmpty)
                  Text(
                    question.detail,
                    style: Theme.of(context).textTheme.bodySmall,
                  ),
              ],
            ),
          ),
        ),
        SecondaryButton(
          label: 'Dismiss',
          shortcut: keys.label(Command.dismissQuestion),
          onPressed: () => state.dismiss(id),
        ),
      ],
    );
    return CallbackShortcuts(
      bindings: keys.bind({
        Command.dismissQuestion: () => state.dismiss(id),
        if (question.kind == BridgeQuestionKind.confirm)
          Command.answer: () => state.answerConfirm(id, true),
      }),
      child: Focus(
        focusNode: _card,
        child: InputCard(
          ring: context.ws.borderStrong,
          padding: const EdgeInsets.all(WsMetrics.space4),
          // A long question scrolls, so its answers stay in the card.
          child: LayoutBuilder(
            builder: (context, card) => Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                if (question.kind == BridgeQuestionKind.select ||
                    question.kind == BridgeQuestionKind.editor) ...[
                  ConstrainedBox(
                    constraints: BoxConstraints(maxHeight: card.maxHeight / 3),
                    child: header,
                  ),
                  const SizedBox(height: WsMetrics.space2),
                  Expanded(child: answer),
                ] else ...[
                  Expanded(child: header),
                  answer,
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }
}
