import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../controls.dart';
import '../rust/api/events.dart';
import '../rust/api/thread.dart';
import '../session/session_state.dart';
import '../shortcuts/keymap.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'question_card.dart';
import 'start_page.dart';

/// Height of the input area; it never changes, so nothing above it moves (guidelines, patterns).
const double actionAreaHeight = 150;

/// Whether `request` asks questions, which take the input area's place; an approval sits in the
/// Chat instead, beside the composer.
bool _asksQuestions(BridgeRequest? request) =>
    request != null && request.approval == null;

/// The input area at the bottom of every tab: the composer, or in its place the agent's
/// questions. Once they are answered, the keys go back to the composer.
class ActionArea extends StatefulWidget {
  const ActionArea({
    super.key,
    required this.state,
    required this.composer,
    required this.onSent,
    this.composerFocus,
  });

  final SessionState state;
  final TextEditingController composer;
  final FocusNode? composerFocus;

  /// Runs when the composer's text is sent, which clears it.
  final VoidCallback onSent;

  @override
  State<ActionArea> createState() => _ActionAreaState();
}

class _ActionAreaState extends State<ActionArea> {
  /// Whether questions were waiting when the request last changed.
  bool _questioning = false;

  @override
  void initState() {
    super.initState();
    _questioning = _asksQuestions(widget.state.request.value);
    widget.state.request.addListener(_changed);
  }

  @override
  void dispose() {
    widget.state.request.removeListener(_changed);
    super.dispose();
  }

  void _changed() {
    final questioning = _asksQuestions(widget.state.request.value);
    if (_questioning && !questioning) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) widget.composerFocus?.requestFocus();
      });
    }
    _questioning = questioning;
  }

  @override
  Widget build(BuildContext context) {
    final state = widget.state;
    return SizedBox(
      height: actionAreaHeight,
      child: ValueListenableBuilder(
        valueListenable: state.request,
        builder: (context, request, _) =>
            request != null && request.approval == null
            ? QuestionCard(
                key: ValueKey(request.id),
                request: request,
                state: state,
              )
            : Composer(
                state: state,
                controller: widget.composer,
                focusNode: widget.composerFocus,
                onSent: widget.onSent,
              ),
      ),
    );
  }
}

/// Text entry for prompts and steering, as T3's composer: ⏎ submits, which steers a running
/// agent, the queue key queues a follow-up and esc stops the run. Queued messages sit above the
/// field.
class Composer extends StatefulWidget {
  const Composer({
    super.key,
    required this.state,
    required this.controller,
    required this.onSent,
    this.focusNode,
  });

  final SessionState state;
  final TextEditingController controller;
  final FocusNode? focusNode;
  final VoidCallback onSent;

  @override
  State<Composer> createState() => _ComposerState();
}

class _ComposerState extends State<Composer> {
  /// The queued message the composer holds for editing, if any.
  BridgeTurn? _editing;

  /// Sends the composer's text: `steer` steers a running agent, else the text queues behind it.
  void _send({required bool steer}) {
    final text = widget.controller.text.trim();
    if (text.isEmpty) return;
    widget.onSent();
    final editing = _editing;
    if (editing != null) {
      setState(() => _editing = null);
      unawaited(widget.state.editQueued(editing, text));
    } else {
      unawaited(widget.state.send(text, steer: steer));
    }
  }

  void _edit(BridgeTurn turn) {
    setState(() => _editing = turn);
    writeText(
      widget.controller,
      TextEditingValue(
        text: turn.prompt,
        selection: TextSelection.collapsed(offset: turn.prompt.length),
      ),
    );
    widget.focusNode?.requestFocus();
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final state = widget.state;
    final controller = widget.controller;
    final keys = Keymap.of(context);
    return InputCard(
      padding: const EdgeInsets.fromLTRB(
        WsMetrics.space5,
        WsMetrics.space4,
        WsMetrics.space4,
        WsMetrics.space4,
      ),
      child: ListenableBuilder(
        listenable: Listenable.merge([
          state.status,
          state.items,
          state.queued,
          state.request,
          controller,
        ]),
        builder: (context, _) {
          final busy = state.status.value != RunStatus.idle;
          final sendable = controller.text.trim().isNotEmpty;
          final approving = state.request.value?.approval != null;
          final queued = state.queued.value;
          final editing = queued
              .where((turn) => turn.number == _editing?.number)
              .firstOrNull;
          // Without a conversation yet, the input area starts a new session.
          final starting = state.items.value.isEmpty;
          // As in T3, a running agent with nothing to send shows only Stop.
          final stopOnly = busy && !sendable && editing == null;
          return Column(
            children: [
              if (queued.isNotEmpty)
                Padding(
                  padding: const EdgeInsets.only(bottom: WsMetrics.space2),
                  child: _Queue(
                    state: state,
                    turns: queued,
                    editing: editing,
                    onEdit: _edit,
                    onCancelEdit: () => setState(() => _editing = null),
                  ),
                ),
              Expanded(
                child: CallbackShortcuts(
                  bindings: keys.bind({
                    Command.send: () => _send(steer: busy),
                    Command.queue: () => _send(steer: false),
                    if (busy) Command.stop: state.stop,
                  }),
                  // A held key acts once; its repeats type no new lines either.
                  child: Focus(
                    canRequestFocus: false,
                    skipTraversal: true,
                    onKeyEvent: (node, event) =>
                        event is KeyRepeatEvent &&
                            [Command.send, Command.queue, Command.stop]
                                .expand(keys.bindings)
                                .any(
                                  (held) => held.repeating.accepts(
                                    event,
                                    HardwareKeyboard.instance,
                                  ),
                                )
                        ? KeyEventResult.handled
                        : KeyEventResult.ignored,
                    child: composingField(
                      controller,
                      TextField(
                        key: const Key('composer'),
                        controller: controller,
                        focusNode: widget.focusNode,
                        maxLines: null,
                        expands: true,
                        style: Theme.of(context).textTheme.bodyLarge,
                        decoration: InputDecoration.collapsed(
                          hintText: approving
                              ? 'Send a message to decline and redirect the agent'
                              : 'Ask anything. @ files, \$ skills, / commands',
                          hintStyle: TextStyle(color: ws.textFaint),
                        ),
                      ),
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
                        if (starting) Flexible(child: StartChips(state: state)),
                      ],
                    ),
                  ),
                  if (stopOnly)
                    Tooltip(
                      message: 'Interrupt',
                      child: SecondaryButton(
                        label: 'Stop generation',
                        shortcut: keys.label(Command.stop),
                        onPressed: state.stop,
                      ),
                    )
                  else ...[
                    if (busy) ...[
                      Hint(keys.hint(Command.queue, 'queue')),
                      const SizedBox(width: WsMetrics.space3),
                    ],
                    PrimaryButton(
                      label: editing != null
                          ? 'Update queued message'
                          : busy
                          ? 'Steer message'
                          : 'Submit message',
                      shortcut: keys.label(Command.send),
                      onPressed: sendable ? () => _send(steer: busy) : null,
                    ),
                  ],
                ],
              ),
            ],
          );
        },
      ),
    );
  }
}

/// The messages waiting behind the running turn, each with where it stands and what it does.
class _Queue extends StatelessWidget {
  const _Queue({
    required this.state,
    required this.turns,
    required this.editing,
    required this.onEdit,
    required this.onCancelEdit,
  });

  final SessionState state;
  final List<BridgeTurn> turns;

  /// The queued message the composer holds for editing, if any.
  final BridgeTurn? editing;
  final ValueChanged<BridgeTurn> onEdit;
  final VoidCallback onCancelEdit;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final small = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
    return ConstrainedBox(
      constraints: const BoxConstraints(maxHeight: 56),
      child: ListView(
        shrinkWrap: true,
        children: [
          for (final (index, turn) in turns.indexed)
            Row(
              children: [
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        'Queued message ${index + 1} of ${turns.length}, not sent',
                        style: small,
                      ),
                      Text(
                        turn.prompt,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          color: ws.textMuted,
                          fontSize: WsMetrics.fsCompact,
                        ),
                      ),
                    ],
                  ),
                ),
                if (turn.number == editing?.number) ...[
                  Text('Editing in the composer', style: small),
                  GhostButton(label: 'Cancel', onPressed: onCancelEdit),
                ] else ...[
                  GhostButton(
                    label: 'Send now',
                    icon: LucideIcons.arrowUp,
                    onPressed: () => unawaited(state.sendQueuedNow(turn)),
                  ),
                  GhostButton(label: 'Edit', onPressed: () => onEdit(turn)),
                  Tooltip(
                    message: 'Remove from queue',
                    child: InkWell(
                      onTap: () => unawaited(state.cancelQueued(turn)),
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
              ],
            ),
        ],
      ),
    );
  }
}
