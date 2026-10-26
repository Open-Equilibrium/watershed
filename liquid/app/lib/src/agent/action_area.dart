import 'package:flutter/material.dart';

import '../controls.dart';
import '../rust/api/session.dart';
import '../session/session_state.dart';
import '../shortcuts/keymap.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'approval_card.dart';
import 'question_card.dart';
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
    return InputCard(
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
                        Tooltip(
                          message: 'Remove from the queue',
                          child: InkWell(
                            key: Key('unqueue $index'),
                            canRequestFocus: false,
                            onTap: () =>
                                state.unqueue(text, followUp: followUp),
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
