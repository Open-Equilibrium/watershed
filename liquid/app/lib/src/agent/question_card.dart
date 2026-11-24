import 'dart:async';

import 'package:flutter/material.dart';

import '../controls.dart';
import '../rust/api/events.dart';
import '../rust/api/session.dart';
import '../session/session_state.dart';
import '../shortcuts/keymap.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'read_delay.dart';

const _advanceDelay = Duration(milliseconds: 200);

class QuestionCard extends StatefulWidget {
  const QuestionCard({super.key, required this.request, required this.state});

  final BridgeRequest request;
  final SessionState state;

  @override
  State<QuestionCard> createState() => _QuestionCardState();
}

class _QuestionCardState extends State<QuestionCard> with ReadDelay {
  int _index = 0;

  final _chosen = <String, List<String>>{};

  final _typed = <String, String>{};
  var _text = TextEditingController();
  final _card = FocusNode(debugLabel: 'question');
  Timer? _advance;

  List<BridgeQuestion> get _questions => widget.request.questions;
  BridgeQuestion get _question => _questions[_index];
  bool get _last => _index == _questions.length - 1;

  List<String> _values(BridgeQuestion question) {
    final typed = (_typed[question.id] ?? '').trim();
    return typed.isNotEmpty ? [typed] : _chosen[question.id] ?? const [];
  }

  bool _isTyped(BridgeQuestion question) =>
      (_typed[question.id] ?? '').trim().isNotEmpty;

  bool get _answered => _values(_question).isNotEmpty;
  bool get _complete => _questions.every((q) => _values(q).isNotEmpty);

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _card.requestFocus();
    });
  }

  @override
  void dispose() {
    _advance?.cancel();
    _text.dispose();
    _card.dispose();
    super.dispose();
  }

  void _go(int index) => setState(() {
    _advance?.cancel();
    _index = index;
    final typed = TextEditingValue(text: _typed[_questions[index].id] ?? '');
    if (writeText(_text, typed)) _text = adoptController(_text);
  });

  void _pick(BridgeQuestion question, String label) {
    setState(() {
      final chosen = _chosen.putIfAbsent(question.id, () => []);
      if (!question.multiSelect) {
        chosen
          ..clear()
          ..add(label);
      } else if (!chosen.remove(label)) {
        chosen.add(label);
      }
    });
    if (question.multiSelect) return;
    _advance?.cancel();
    _advance = Timer(_advanceDelay, () {
      if (mounted) _primary();
    });
  }

  void _primary() {
    if (_last) {
      if (_complete) _send();
    } else if (_answered) {
      _go(_index + 1);
    }
  }

  void _send() {
    _advance?.cancel();
    decided(
      () => widget.state.answer(widget.request, [
        for (final question in _questions)
          BridgeQuestionAnswer(
            question: question.id,
            values: _values(question),
            typed: _isTyped(question),
          ),
      ]),
    );
  }

  void _dismiss() {
    _advance?.cancel();
    decided(() => widget.state.dismiss(widget.request));
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final keys = Keymap.of(context);
    final question = _question;
    final count = _questions.length;
    final ready = _last ? _complete : _answered;
    final label = !_last
        ? 'Next question'
        : count > 1
        ? 'Submit answers'
        : 'Submit answer';
    return CallbackShortcuts(
      bindings: keys.bind({
        Command.answer: () => decided(_primary),
        Command.dismissQuestion: _dismiss,
      }),
      child: Focus(
        focusNode: _card,
        onKeyEvent: skipHeld,
        child: InputCard(
          ring: ws.borderStrong,
          padding: const EdgeInsets.all(WsMetrics.space4),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Row(
                children: [
                  Expanded(
                    child: Text(
                      question.header,
                      style: Theme.of(context).textTheme.labelMedium,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
                  if (count > 1)
                    Text(
                      '${_index + 1}/$count',
                      style: TextStyle(
                        color: ws.textFaint,
                        fontSize: WsMetrics.fsSmall,
                      ),
                    ),
                  const SizedBox(width: WsMetrics.space3),
                  Tooltip(
                    message: 'Dismiss question without answering',
                    child: InkWell(
                      onTap: _dismiss,
                      child: Icon(LucideIcons.x, size: 14, color: ws.textFaint),
                    ),
                  ),
                ],
              ),
              const SizedBox(height: WsMetrics.space1),
              Expanded(
                child: SingleChildScrollView(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      Text(
                        question.question,
                        style: Theme.of(context).textTheme.bodyMedium,
                      ),
                      for (final (index, option) in question.options.indexed)
                        _Option(
                          label: option,
                          description: index < question.descriptions.length
                              ? question.descriptions[index]
                              : null,
                          multi: question.multiSelect,
                          chosen:
                              _chosen[question.id]?.contains(option) ?? false,
                          onTap: () => decided(() => _pick(question, option)),
                        ),
                      if (question.custom)
                        composingField(
                          _text,
                          TextField(
                            key: const Key('answer'),
                            controller: _text,
                            onEditingComplete: () {},
                            onChanged: (value) =>
                                setState(() => _typed[question.id] = value),
                            decoration: InputDecoration.collapsed(
                              hintText: question.options.isEmpty
                                  ? 'Type your answer'
                                  : 'Type your own answer, or leave this blank to use the selected option',
                              hintStyle: TextStyle(color: ws.textFaint),
                            ),
                          ),
                        ),
                    ],
                  ),
                ),
              ),
              const SizedBox(height: WsMetrics.space2),
              Row(
                children: [
                  if (_index > 0)
                    SecondaryButton(
                      label: 'Previous',
                      onPressed: () => decided(() => _go(_index - 1)),
                    ),
                  const Spacer(),
                  PrimaryButton(
                    label: label,
                    shortcut: keys.label(Command.answer),
                    onPressed: ready ? () => decided(_primary) : null,
                  ),
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _Option extends StatelessWidget {
  const _Option({
    required this.label,
    required this.description,
    required this.multi,
    required this.chosen,
    required this.onTap,
  });

  final String label;
  final String? description;
  final bool multi;
  final bool chosen;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final text = description;
    return InkWell(
      onTap: onTap,
      borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
      child: Container(
        padding: const EdgeInsets.symmetric(
          horizontal: WsMetrics.space2,
          vertical: WsMetrics.space1,
        ),
        decoration: BoxDecoration(
          color: chosen ? ws.selected : null,
          borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        ),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Container(
              width: 14,
              height: 14,
              margin: const EdgeInsets.only(top: 2),
              decoration: BoxDecoration(
                shape: multi ? BoxShape.rectangle : BoxShape.circle,
                borderRadius: multi ? BorderRadius.circular(3) : null,
                color: chosen ? ws.accent : null,
                border: Border.all(color: ws.borderStrong),
              ),
              child: !chosen
                  ? null
                  : multi
                  ? Icon(LucideIcons.check, size: 10, color: ws.onAccent)
                  : Center(
                      child: Container(
                        width: 6,
                        height: 6,
                        decoration: BoxDecoration(
                          shape: BoxShape.circle,
                          color: ws.onAccent,
                        ),
                      ),
                    ),
            ),
            const SizedBox(width: WsMetrics.space3),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    label,
                    style: const TextStyle(fontWeight: WsMetrics.fwMedium),
                  ),
                  if (text != null && text.isNotEmpty && text != label)
                    Text(
                      text,
                      style: TextStyle(
                        color: ws.textMuted,
                        fontSize: WsMetrics.fsSmall,
                      ),
                    ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}
