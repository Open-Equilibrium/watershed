import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../rust/api/events.dart';
import '../rust/api/session.dart';
import '../session/session_state.dart';
import '../shortcuts/keymap.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'shortcut_reveal.dart';
import 'start_page.dart';

/// Height of the input area; it never changes, so nothing above it moves (guidelines, patterns).
const double actionAreaHeight = 150;

/// The input area at the bottom of every tab: the composer, or in its place an approval or a
/// question waiting for the user.
class ActionArea extends StatelessWidget {
  const ActionArea({
    super.key,
    required this.state,
    required this.composer,
    required this.project,
    this.composerFocus,
  });

  final SessionState state;
  final TextEditingController composer;
  final FocusNode? composerFocus;

  /// The project's name, as a new session's chips show it.
  final String project;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      height: actionAreaHeight,
      child: ValueListenableBuilder(
        valueListenable: state.approval,
        builder: (context, approval, _) {
          if (approval != null) {
            return ApprovalCard(
              key: ValueKey(approval.id),
              request: approval,
              state: state,
            );
          }
          return ValueListenableBuilder(
            valueListenable: state.question,
            builder: (context, question, _) => question != null
                ? QuestionCard(
                    key: ValueKey(question.id),
                    request: question,
                    state: state,
                  )
                : Composer(
                    state: state,
                    controller: composer,
                    focusNode: composerFocus,
                    project: project,
                  ),
          );
        },
      ),
    );
  }
}

BoxDecoration _box(BuildContext context, {Color? ring}) => BoxDecoration(
  color: context.ws.surface,
  borderRadius: BorderRadius.circular(WsMetrics.radiusXl),
  boxShadow: [
    if (ring != null) BoxShadow(color: ring, spreadRadius: 1),
    ...context.ws.shadow,
  ],
);

/// Text entry for prompts and steering (approved session-flow mockup): Enter sends or steers a
/// running agent, ⌥Enter queues a follow-up, Esc stops the run, Shift+Enter adds a line.
/// Waiting messages are chips inside the input area.
class Composer extends StatelessWidget {
  const Composer({
    super.key,
    required this.state,
    required this.controller,
    required this.project,
    this.focusNode,
  });

  final SessionState state;
  final TextEditingController controller;
  final FocusNode? focusNode;
  final String project;

  void _send({required bool followUp}) {
    final text = controller.text.trim();
    if (text.isEmpty) return;
    controller.clear();
    state.send(text, followUp: followUp);
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      decoration: _box(context),
      padding: const EdgeInsets.fromLTRB(
        WsMetrics.space5,
        WsMetrics.space4,
        WsMetrics.space4,
        WsMetrics.space4,
      ),
      child: ListenableBuilder(
        listenable: Listenable.merge([state.status, state.items]),
        builder: (context, _) {
          final running = state.status.value == RunStatus.running;
          // Without a conversation yet, the input area starts a new session (ADR-0203).
          final starting = !running && state.items.value.isEmpty;
          final keys = Keymap.of(context);
          return Column(
            children: [
              _QueueChips(state: state),
              Expanded(
                child: CallbackShortcuts(
                  bindings: keys.bind({
                    Command.send: () => _send(followUp: false),
                    Command.queue: () => _send(followUp: true),
                    if (running) Command.stop: state.stop,
                  }),
                  child: TextField(
                    key: const Key('composer'),
                    controller: controller,
                    focusNode: focusNode,
                    maxLines: null,
                    expands: true,
                    style: Theme.of(context).textTheme.bodyLarge,
                    decoration: InputDecoration.collapsed(
                      hintText: starting
                          ? 'Describe the task…'
                          : 'Steer the agent or queue a follow-up…',
                      hintStyle: TextStyle(color: ws.textFaint),
                    ),
                  ),
                ),
              ),
              Row(
                children: [
                  // The setup chips shorten long names, so the buttons stay in view.
                  Expanded(
                    child: Row(
                      children: [
                        if (starting) ...[
                          Flexible(
                            child: StartChips(state: state, project: project),
                          ),
                          const SizedBox(width: WsMetrics.space2),
                        ],
                        ModeChip(state: state),
                      ],
                    ),
                  ),
                  if (running) ...[
                    Hint(keys.hint(Command.queue, 'queue')),
                    const SizedBox(width: WsMetrics.space3),
                    SecondaryButton(
                      label: 'Stop',
                      shortcut: keys.label(Command.stop),
                      onPressed: state.stop,
                    ),
                  ],
                  const SizedBox(width: WsMetrics.space3),
                  PrimaryButton(
                    label: running
                        ? 'Steer'
                        : starting
                        ? 'Start'
                        : 'Send',
                    shortcut: keys.label(Command.send),
                    onPressed: () => _send(followUp: false),
                  ),
                ],
              ),
            ],
          );
        },
      ),
    );
  }
}

/// Messages waiting for the running agent; ✕ takes one out.
class _QueueChips extends StatelessWidget {
  const _QueueChips({required this.state});

  final SessionState state;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return ValueListenableBuilder(
      valueListenable: state.queue,
      builder: (context, queue, _) {
        final waiting = [
          for (final text in queue.steering) ('Steer', text, false),
          for (final text in queue.followUp) ('Queued', text, true),
        ];
        if (waiting.isEmpty) return const SizedBox.shrink();
        return Padding(
          padding: const EdgeInsets.only(bottom: WsMetrics.space2),
          child: SingleChildScrollView(
            scrollDirection: Axis.horizontal,
            child: Row(
              children: [
                for (final (index, (kind, text, followUp)) in waiting.indexed)
                  Container(
                    height: 22,
                    margin: const EdgeInsets.only(right: WsMetrics.space2),
                    padding: const EdgeInsets.only(left: WsMetrics.space2),
                    decoration: BoxDecoration(
                      color: ws.subtle,
                      borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                    ),
                    child: Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        ConstrainedBox(
                          constraints: const BoxConstraints(maxWidth: 260),
                          child: Text(
                            '$kind: $text',
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: TextStyle(
                              color: ws.textMuted,
                              fontSize: WsMetrics.fsSmall,
                            ),
                          ),
                        ),
                        InkWell(
                          key: Key('unqueue $index'),
                          canRequestFocus: false,
                          onTap: () => state.unqueue(text, followUp: followUp),
                          child: Padding(
                            padding: const EdgeInsets.symmetric(
                              horizontal: WsMetrics.space2,
                            ),
                            child: Icon(
                              LucideIcons.x,
                              size: 12,
                              color: ws.textFaint,
                            ),
                          ),
                        ),
                      ],
                    ),
                  ),
              ],
            ),
          ),
        );
      },
    );
  }
}

/// Chooses how much the agent may do without asking (ADR-0188).
class ModeChip extends StatelessWidget {
  const ModeChip({super.key, required this.state});

  final SessionState state;

  static const labels = {
    BridgeApprovalMode.ask: 'Ask',
    BridgeApprovalMode.allowEdits: 'Allow edits',
    BridgeApprovalMode.fullAccess: 'Full access',
  };

  @override
  Widget build(BuildContext context) {
    return ValueListenableBuilder(
      valueListenable: state.mode,
      builder: (context, mode, _) => PopupMenuButton<BridgeApprovalMode>(
        tooltip: 'Approval mode',
        initialValue: mode,
        onSelected: state.setMode,
        itemBuilder: (context) => [
          for (final entry in labels.entries)
            PopupMenuItem(value: entry.key, child: Text(entry.value)),
        ],
        child: Container(
          height: 28,
          padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
          decoration: BoxDecoration(
            border: Border.all(color: context.ws.borderStrong),
            borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
          ),
          alignment: Alignment.center,
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(LucideIcons.shield, size: 13, color: context.ws.textMuted),
              const SizedBox(width: WsMetrics.space1),
              Text(
                labels[mode]!,
                style: TextStyle(color: context.ws.textMuted, fontSize: 13),
              ),
              const SizedBox(width: WsMetrics.space1),
              Icon(
                LucideIcons.chevronDown,
                size: 13,
                color: context.ws.textFaint,
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// A tool call waiting for approval: Enter allows once, Cmd+Enter for the session, Esc rejects.
class ApprovalCard extends StatefulWidget {
  const ApprovalCard({super.key, required this.request, required this.state});

  final BridgeRequest request;
  final SessionState state;

  @override
  State<ApprovalCard> createState() => _ApprovalCardState();
}

class _ApprovalCardState extends State<ApprovalCard> {
  final _focus = FocusNode(debugLabel: 'approval');

  @override
  void initState() {
    super.initState();
    // The card takes the keyboard as soon as it appears, so the decision needs no mouse.
    WidgetsBinding.instance.addPostFrameCallback((_) => _focus.requestFocus());
  }

  @override
  void dispose() {
    _focus.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final state = widget.state;
    final approval = widget.request.approval!;
    final ws = context.ws;
    final title = switch (approval.kind) {
      BridgeRequestKind.commandExecution => 'Run a shell command?',
      BridgeRequestKind.fileChange => 'Change project files?',
      BridgeRequestKind.tool => 'Call ${approval.tool}?',
    };
    final keys = Keymap.of(context);
    return CallbackShortcuts(
      bindings: keys.bind({
        Command.allowOnce: () => state.decide(BridgeDecision.allowOnce),
        Command.allowSession: () => state.decide(BridgeDecision.allowSession),
        Command.reject: () => state.decide(BridgeDecision.reject),
      }),
      child: Focus(
        focusNode: _focus,
        child: Container(
          decoration: _box(context, ring: ws.warningBorder),
          padding: const EdgeInsets.all(WsMetrics.space4),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Row(
                children: [
                  StatusBadge(
                    text: '!',
                    color: ws.warning,
                    background: ws.warningSubtle,
                  ),
                  const SizedBox(width: WsMetrics.space4),
                  Text(title, style: Theme.of(context).textTheme.labelMedium),
                  const Spacer(),
                  Text(
                    approval.tool,
                    style: Theme.of(context).textTheme.bodySmall,
                  ),
                ],
              ),
              const SizedBox(height: WsMetrics.space3),
              Expanded(child: _ScrollingCode(_call(approval))),
              const SizedBox(height: WsMetrics.space3),
              Row(
                children: [
                  PrimaryButton(
                    label: 'Allow once',
                    shortcut: keys.label(Command.allowOnce),
                    onPressed: () => state.decide(BridgeDecision.allowOnce),
                  ),
                  const SizedBox(width: WsMetrics.space3),
                  SecondaryButton(
                    label: 'Allow for this session',
                    shortcut: keys.label(Command.allowSession),
                    onPressed: () => state.decide(BridgeDecision.allowSession),
                  ),
                  Expanded(
                    child: Text(
                      'your draft is kept',
                      textAlign: TextAlign.end,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        color: ws.textFaint,
                        fontSize: WsMetrics.fsSmall,
                      ),
                    ),
                  ),
                  const SizedBox(width: WsMetrics.space3),
                  SecondaryButton(
                    label: 'Reject',
                    shortcut: keys.label(Command.reject),
                    color: ws.danger,
                    onPressed: () => state.decide(BridgeDecision.reject),
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

/// What an approval shows: a shell command itself, other tools their arguments.
String _call(BridgeApproval approval) {
  if (approval.tool == 'bash') {
    try {
      if (jsonDecode(approval.arguments) case {
        'command': final String command,
      }) {
        return command;
      }
    } on FormatException {
      // Shown as the agent sent it.
    }
  }
  return approval.arguments;
}

/// Code that scrolls inside a fixed height and says how many lines are below, so nothing is
/// hidden without a hint (approved approval mockup).
class _ScrollingCode extends StatefulWidget {
  const _ScrollingCode(this.text);

  final String text;

  @override
  State<_ScrollingCode> createState() => _ScrollingCodeState();
}

class _ScrollingCodeState extends State<_ScrollingCode> {
  int _below = 0;

  bool _measure(ScrollMetrics metrics, double lineHeight) {
    final below = (metrics.extentAfter / lineHeight).ceil();
    if (below != _below) setState(() => _below = below);
    return false;
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final style = monoStyle(context);
    final lineHeight = (TextPainter(
      text: TextSpan(text: ' ', style: style),
      textDirection: TextDirection.ltr,
    )..layout()).height;
    return Container(
      decoration: BoxDecoration(
        color: ws.code,
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
      ),
      child: Stack(
        children: [
          NotificationListener<ScrollMetricsNotification>(
            onNotification: (notification) =>
                _measure(notification.metrics, lineHeight),
            child: NotificationListener<ScrollUpdateNotification>(
              onNotification: (notification) =>
                  _measure(notification.metrics, lineHeight),
              child: SingleChildScrollView(
                padding: const EdgeInsets.symmetric(
                  horizontal: WsMetrics.space4,
                  vertical: WsMetrics.space2,
                ),
                child: SizedBox(
                  width: double.infinity,
                  child: Text(widget.text, style: style),
                ),
              ),
            ),
          ),
          if (_below > 0)
            Positioned(
              right: WsMetrics.space3,
              bottom: WsMetrics.space1,
              child: Container(
                color: ws.code,
                padding: const EdgeInsets.only(left: WsMetrics.space2),
                child: Text(
                  '↓ $_below more line${_below == 1 ? '' : 's'}',
                  style: TextStyle(
                    color: ws.textFaint,
                    fontSize: WsMetrics.fsSmall,
                  ),
                ),
              ),
            ),
        ],
      ),
    );
  }
}

/// A selection that narrows as the user types; arrows move and Enter picks (approved mockup).
class _OptionList extends StatefulWidget {
  const _OptionList({required this.options, required this.onPick});

  final List<String> options;
  final ValueChanged<String> onPick;

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
                    const SingleActivator(LogicalKeyboardKey.arrowDown): () =>
                        _move(1),
                    const SingleActivator(LogicalKeyboardKey.arrowUp): () =>
                        _move(-1),
                    for (final keys in Keymap.of(
                      context,
                    ).bindings(Command.answer))
                      keys.activator: _pick,
                  },
                  child: TextField(
                    key: const Key('filter'),
                    controller: _filter,
                    autofocus: true,
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

  @override
  void dispose() {
    _text.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final question = widget.request.question!;
    final state = widget.state;
    final keys = Keymap.of(context);
    final answer = switch (question.kind) {
      BridgeQuestionKind.select => _OptionList(
        options: question.options,
        onPick: state.answerText,
      ),
      BridgeQuestionKind.confirm => Row(
        children: [
          PrimaryButton(
            label: 'Yes',
            shortcut: keys.label(Command.answer),
            onPressed: () => state.answerConfirm(true),
          ),
          const SizedBox(width: WsMetrics.space3),
          SecondaryButton(
            label: 'No',
            onPressed: () => state.answerConfirm(false),
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
                Command.answerEditor: () => state.answerText(_text.text),
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
                  autofocus: true,
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
                onPressed: () => state.answerText(_text.text),
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
                Command.answer: () => state.answerText(_text.text),
              }),
              child: TextField(
                key: const Key('answer'),
                controller: _text,
                autofocus: true,
                decoration: InputDecoration.collapsed(
                  hintText: question.detail,
                ),
                onSubmitted: (text) => state.answerText(text),
              ),
            ),
          ),
          PrimaryButton(
            label: 'Answer',
            shortcut: keys.label(Command.answer),
            onPressed: () => state.answerText(_text.text),
          ),
        ],
      ),
    };
    return CallbackShortcuts(
      bindings: keys.bind({
        Command.dismissQuestion: state.dismiss,
        if (question.kind == BridgeQuestionKind.confirm)
          Command.answer: () => state.answerConfirm(true),
      }),
      child: Focus(
        // Text questions focus their field; a confirmation takes the keys itself.
        autofocus: question.kind == BridgeQuestionKind.confirm,
        child: Container(
          decoration: _box(context, ring: context.ws.borderStrong),
          padding: const EdgeInsets.all(WsMetrics.space4),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Row(
                children: [
                  Expanded(
                    child: Text(
                      question.title,
                      style: Theme.of(context).textTheme.labelMedium,
                    ),
                  ),
                  SecondaryButton(
                    label: 'Dismiss',
                    shortcut: keys.label(Command.dismissQuestion),
                    onPressed: state.dismiss,
                  ),
                ],
              ),
              if (question.kind == BridgeQuestionKind.confirm &&
                  question.detail.isNotEmpty)
                Text(
                  question.detail,
                  style: Theme.of(context).textTheme.bodySmall,
                ),
              if (question.kind == BridgeQuestionKind.select ||
                  question.kind == BridgeQuestionKind.editor) ...[
                const SizedBox(height: WsMetrics.space2),
                Expanded(child: answer),
              ] else ...[
                const Spacer(),
                answer,
              ],
            ],
          ),
        ),
      ),
    );
  }
}

/// A contextual shortcut hint, shown only while the reveal key is held (ADR-0204). It takes no space, so
/// nothing moves when it appears.
class Hint extends StatelessWidget {
  const Hint(this.text, {super.key});

  final String text;

  @override
  Widget build(BuildContext context) {
    if (text.isEmpty || !ShortcutReveal.shown(context)) {
      return const SizedBox.shrink();
    }
    return SizedBox.shrink(
      child: OverflowBox(
        alignment: Alignment.centerLeft,
        maxWidth: double.infinity,
        maxHeight: double.infinity,
        child: KeyBadge(text),
      ),
    );
  }
}

/// The look of a shortcut hint: small, dark and above what it labels (approved mockup).
class KeyBadge extends StatelessWidget {
  const KeyBadge(this.text, {super.key});

  final String text;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space1),
      decoration: BoxDecoration(
        color: ws.text,
        borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
        boxShadow: ws.shadow,
      ),
      child: Text(
        text,
        style: TextStyle(
          fontSize: 10,
          height: 14 / 10,
          fontWeight: WsMetrics.fwSemibold,
          color: ws.surface,
        ),
      ),
    );
  }
}

/// A one-character status badge.
class StatusBadge extends StatelessWidget {
  const StatusBadge({
    super.key,
    required this.text,
    required this.color,
    required this.background,
  });

  final String text;
  final Color color;
  final Color background;

  @override
  Widget build(BuildContext context) => Container(
    width: 22,
    height: 22,
    alignment: Alignment.center,
    decoration: BoxDecoration(
      color: background,
      borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
    ),
    child: Text(
      text,
      style: TextStyle(
        color: color,
        fontWeight: WsMetrics.fwBold,
        fontSize: WsMetrics.fsSmall,
      ),
    ),
  );
}

class _Button extends StatelessWidget {
  const _Button({
    required this.label,
    required this.onPressed,
    required this.foreground,
    required this.background,
    this.outline,
    this.shortcut,
  });

  final String label;
  final String? shortcut;
  final VoidCallback onPressed;
  final Color foreground;
  final Color background;
  final Color? outline;

  @override
  Widget build(BuildContext context) {
    final button = Material(
      color: background,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        side: outline == null ? BorderSide.none : BorderSide(color: outline!),
      ),
      child: InkWell(
        onTap: onPressed,
        canRequestFocus: false,
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        child: Container(
          height: 28,
          padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space4),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Text(
                label,
                style: TextStyle(
                  color: foreground,
                  fontWeight: WsMetrics.fwMedium,
                  fontSize: 13,
                ),
              ),
            ],
          ),
        ),
      ),
    );
    final hint = shortcut;
    if (hint == null || hint.isEmpty || !ShortcutReveal.shown(context)) {
      return button;
    }
    // The hint sits on the button's corner, so the button keeps its size.
    return Stack(
      clipBehavior: Clip.none,
      children: [
        button,
        Positioned(top: -9, right: -6, child: KeyBadge(hint)),
      ],
    );
  }
}

class PrimaryButton extends StatelessWidget {
  const PrimaryButton({
    super.key,
    required this.label,
    required this.onPressed,
    this.shortcut,
  });

  final String label;
  final String? shortcut;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) => _Button(
    label: label,
    shortcut: shortcut,
    onPressed: onPressed,
    foreground: context.ws.onAccent,
    background: context.ws.accent,
  );
}

class SecondaryButton extends StatelessWidget {
  const SecondaryButton({
    super.key,
    required this.label,
    required this.onPressed,
    this.shortcut,
    this.color,
  });

  final String label;
  final String? shortcut;
  final Color? color;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) => _Button(
    label: label,
    shortcut: shortcut,
    onPressed: onPressed,
    foreground: color ?? context.ws.text,
    background: context.ws.canvas,
    outline: context.ws.borderStrong,
  );
}
