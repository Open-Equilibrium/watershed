import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../controls.dart';
import '../rust/api/events.dart';
import '../rust/api/session.dart';
import '../session/session_state.dart';
import '../shortcuts/keymap.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'start_page.dart';

/// Height of the input area; it never changes, so nothing above it moves (guidelines, patterns).
const double actionAreaHeight = 150;

/// The input area at the bottom of every tab: the composer, or in its place an approval or a
/// question waiting for the user. Once it is answered, the keys go back to the composer.
class ActionArea extends StatefulWidget {
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
  State<ActionArea> createState() => _ActionAreaState();
}

class _ActionAreaState extends State<ActionArea> {
  late final _waiting = Listenable.merge([
    widget.state.approval,
    widget.state.question,
  ]);

  /// Whether a card was shown when the requests last changed.
  bool _asking = false;

  bool get _shown =>
      widget.state.approval.value != null ||
      widget.state.question.value != null;

  @override
  void initState() {
    super.initState();
    _asking = _shown;
    _waiting.addListener(_changed);
  }

  @override
  void dispose() {
    _waiting.removeListener(_changed);
    super.dispose();
  }

  void _changed() {
    final asking = _shown;
    if (_asking && !asking) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) widget.composerFocus?.requestFocus();
      });
    }
    _asking = asking;
  }

  @override
  Widget build(BuildContext context) {
    final state = widget.state;
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
                    controller: widget.composer,
                    focusNode: widget.composerFocus,
                    project: widget.project,
                  ),
          );
        },
      ),
    );
  }
}

/// The input area's card; controls in it show focus and hover above its background.
class _Box extends StatelessWidget {
  const _Box({this.ring, required this.padding, required this.child});

  final Color? ring;
  final EdgeInsetsGeometry padding;
  final Widget child;

  @override
  Widget build(BuildContext context) => Container(
    decoration: BoxDecoration(
      color: context.ws.surface,
      borderRadius: BorderRadius.circular(WsMetrics.radiusXl),
      boxShadow: [
        if (ring case final ring?) BoxShadow(color: ring, spreadRadius: 1),
        ...context.ws.shadow,
      ],
    ),
    padding: padding,
    child: Material(type: MaterialType.transparency, child: child),
  );
}

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
    return _Box(
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

  /// What each mode lets the agent do, and what Allow edits risks (approved mockup, ADR-0206).
  static const _explained = {
    BridgeApprovalMode.ask: (
      'Asks before every call that is not read-only.',
      null,
    ),
    BridgeApprovalMode.allowEdits: (
      'Changes files without asking.',
      'Any file you can change, including settings that run commands, such as Git settings '
          'Liquid uses for the diff.',
    ),
    BridgeApprovalMode.fullAccess: ('Runs every call without asking.', null),
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
            PopupMenuItem(
              value: entry.key,
              child: _ModeItem(
                name: entry.value,
                explained: _explained[entry.key]!,
              ),
            ),
        ],
        child: Container(
          height: 28,
          padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
          decoration: BoxDecoration(
            border: Border.all(color: context.ws.borderStrong),
            borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
          ),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(LucideIcons.shield, size: 13, color: context.ws.textMuted),
              const SizedBox(width: WsMetrics.space1),
              Text(
                labels[mode]!,
                style: TextStyle(
                  color: context.ws.textMuted,
                  fontSize: WsMetrics.fsCompact,
                ),
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

/// One mode in the mode menu: its name, what it does and, for Allow edits, its risk.
class _ModeItem extends StatelessWidget {
  const _ModeItem({required this.name, required this.explained});

  final String name;
  final (String, String?) explained;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final (description, risk) = explained;
    final small = TextStyle(fontSize: WsMetrics.fsSmall, color: ws.textMuted);
    return ConstrainedBox(
      constraints: const BoxConstraints(maxWidth: 280),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(name, style: const TextStyle(fontWeight: WsMetrics.fwMedium)),
          Text(description, style: small),
          if (risk != null)
            Padding(
              padding: const EdgeInsets.only(top: 2),
              child: Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Icon(LucideIcons.triangleAlert, size: 12, color: ws.warning),
                  const SizedBox(width: WsMetrics.space1),
                  Expanded(
                    child: Text(risk, style: small.copyWith(color: ws.warning)),
                  ),
                ],
              ),
            ),
        ],
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
  final _code = ScrollController();

  /// Whether the card was shown long enough to be read; until then no key decides (ADR-0207).
  bool _armed = false;
  Timer? _arming;

  /// Starts the wait again whenever Liquid comes back in front, since the card may have
  /// appeared while the user worked elsewhere.
  late final AppLifecycleListener _front;

  @override
  void initState() {
    super.initState();
    // The card takes the keyboard as soon as it appears, so the decision needs no mouse.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _focus.requestFocus();
    });
    _front = AppLifecycleListener(onStateChange: (_) => _arm());
    _arm();
  }

  /// Takes keys 0.5 s after Liquid is in front with the card.
  void _arm() {
    _armed = false;
    _arming?.cancel();
    if (WidgetsBinding.instance.lifecycleState case null ||
        AppLifecycleState.resumed) {
      _arming = Timer(const Duration(milliseconds: 500), () => _armed = true);
    }
  }

  @override
  void dispose() {
    _arming?.cancel();
    _front.dispose();
    _focus.dispose();
    _code.dispose();
    super.dispose();
  }

  /// Moves the call by a line, or by a page of its view.
  void _scroll(int direction, {bool page = false}) {
    if (!_code.hasClients) return;
    final position = _code.position;
    final step = page
        ? position.viewportDimension
        : _lineHeight(monoStyle(context));
    _code.jumpTo(
      (position.pixels + direction * step).clamp(
        position.minScrollExtent,
        position.maxScrollExtent,
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final state = widget.state;
    final id = widget.request.id;
    void decide(BridgeDecision decision) => state.decide(id, decision);
    final approval = widget.request.approval!;
    final ws = context.ws;
    final title = switch (approval.kind) {
      BridgeRequestKind.commandExecution => 'Run a shell command?',
      BridgeRequestKind.fileChange => 'Change files?',
      BridgeRequestKind.tool => 'Call ${approval.tool}?',
    };
    final keys = Keymap.of(context);
    final decisions = {
      Command.allowOnce: () => decide(BridgeDecision.allowOnce),
      Command.allowSession: () => decide(BridgeDecision.allowSession),
      Command.reject: () => decide(BridgeDecision.reject),
    };
    return CallbackShortcuts(
      bindings: {
        ...keys.bind(decisions),
        const SingleActivator(LogicalKeyboardKey.arrowUp): () => _scroll(-1),
        const SingleActivator(LogicalKeyboardKey.arrowDown): () => _scroll(1),
        const SingleActivator(LogicalKeyboardKey.pageUp): () =>
            _scroll(-1, page: true),
        const SingleActivator(LogicalKeyboardKey.pageDown): () =>
            _scroll(1, page: true),
      },
      child: Focus(
        focusNode: _focus,
        // A key pressed before the card was read, or held from before, decides nothing.
        onKeyEvent: (node, event) =>
            (!_armed || event is KeyRepeatEvent) &&
                decisions.keys
                    .expand(keys.bindings)
                    .any(
                      (decision) => decision.activator.accepts(
                        event,
                        HardwareKeyboard.instance,
                      ),
                    )
            ? KeyEventResult.handled
            : KeyEventResult.ignored,
        child: _Box(
          ring: ws.warningBorder,
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
              Expanded(child: _ScrollingCode(approval.text, controller: _code)),
              const SizedBox(height: WsMetrics.space3),
              Row(
                children: [
                  PrimaryButton(
                    label: 'Allow once',
                    shortcut: keys.label(Command.allowOnce),
                    onPressed: () => decide(BridgeDecision.allowOnce),
                  ),
                  const SizedBox(width: WsMetrics.space3),
                  SecondaryButton(
                    label: 'Allow for this session',
                    shortcut: keys.label(Command.allowSession),
                    onPressed: () => decide(BridgeDecision.allowSession),
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
                    onPressed: () => decide(BridgeDecision.reject),
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

/// Code that scrolls inside a fixed height and says how many lines are below, so nothing is
/// hidden without a hint (approved approval mockup).
class _ScrollingCode extends StatefulWidget {
  const _ScrollingCode(this.text, {required this.controller});

  final String text;
  final ScrollController controller;

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
    final lineHeight = _lineHeight(style);
    return Container(
      decoration: BoxDecoration(
        color: ws.code,
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
      ),
      // The hint sits below the code, so it never covers a line of the call.
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Expanded(
            child: NotificationListener<ScrollMetricsNotification>(
              onNotification: (notification) =>
                  _measure(notification.metrics, lineHeight),
              child: NotificationListener<ScrollUpdateNotification>(
                onNotification: (notification) =>
                    _measure(notification.metrics, lineHeight),
                child: SingleChildScrollView(
                  controller: widget.controller,
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
          ),
          if (_below > 0)
            Padding(
              padding: const EdgeInsets.only(
                right: WsMetrics.space3,
                bottom: WsMetrics.space1,
              ),
              child: Text(
                '↓ $_below more line${_below == 1 ? '' : 's'}',
                textAlign: TextAlign.end,
                style: TextStyle(
                  color: ws.textFaint,
                  fontSize: WsMetrics.fsSmall,
                ),
              ),
            ),
        ],
      ),
    );
  }
}

double _lineHeight(TextStyle style) => (TextPainter(
  text: TextSpan(text: ' ', style: style),
  textDirection: TextDirection.ltr,
)..layout()).height;

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
    return CallbackShortcuts(
      bindings: keys.bind({
        Command.dismissQuestion: () => state.dismiss(id),
        if (question.kind == BridgeQuestionKind.confirm)
          Command.answer: () => state.answerConfirm(id, true),
      }),
      child: Focus(
        focusNode: _card,
        child: _Box(
          ring: context.ws.borderStrong,
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
                    onPressed: () => state.dismiss(id),
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
