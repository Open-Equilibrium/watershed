import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../rust/api/events.dart';
import '../rust/api/session.dart';
import '../session/session_state.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';

/// Height of the input area; it never changes, so nothing above it moves (guidelines, patterns).
const double actionAreaHeight = 120;

/// The input area at the bottom of every tab: the composer, or in its place an approval or a
/// question waiting for the user.
class ActionArea extends StatelessWidget {
  const ActionArea({
    super.key,
    required this.state,
    required this.composer,
    this.composerFocus,
  });

  final SessionState state;
  final TextEditingController composer;
  final FocusNode? composerFocus;

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
    this.focusNode,
  });

  final SessionState state;
  final TextEditingController controller;
  final FocusNode? focusNode;

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
      child: ValueListenableBuilder(
        valueListenable: state.status,
        builder: (context, status, _) {
          final running = status == RunStatus.running;
          return Column(
            children: [
              _QueueChips(state: state),
              Expanded(
                child: CallbackShortcuts(
                  bindings: {
                    const SingleActivator(LogicalKeyboardKey.enter): () =>
                        _send(followUp: false),
                    const SingleActivator(
                      LogicalKeyboardKey.enter,
                      alt: true,
                    ): () =>
                        _send(followUp: true),
                    if (running)
                      const SingleActivator(LogicalKeyboardKey.escape):
                          state.stop,
                  },
                  child: TextField(
                    key: const Key('composer'),
                    controller: controller,
                    focusNode: focusNode,
                    maxLines: null,
                    expands: true,
                    style: Theme.of(context).textTheme.bodyLarge,
                    decoration: InputDecoration.collapsed(
                      hintText: 'Steer the agent or queue a follow-up…',
                      hintStyle: TextStyle(color: ws.textFaint),
                    ),
                  ),
                ),
              ),
              Row(
                children: [
                  ModeChip(state: state),
                  const Spacer(),
                  if (running) ...[
                    Text(
                      '⌥⏎ queue',
                      style: TextStyle(
                        color: ws.textFaint,
                        fontSize: WsMetrics.fsSmall,
                      ),
                    ),
                    const SizedBox(width: WsMetrics.space3),
                    SecondaryButton(
                      label: 'Stop',
                      shortcut: 'esc',
                      onPressed: state.stop,
                    ),
                  ] else
                    Hint('⌘L'),
                  const SizedBox(width: WsMetrics.space3),
                  PrimaryButton(
                    label: running ? 'Steer' : 'Send',
                    shortcut: '⏎',
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
          for (final text in queue.steering) ('Steer', text),
          for (final text in queue.followUp) ('Queued', text),
        ];
        if (waiting.isEmpty) return const SizedBox.shrink();
        return Padding(
          padding: const EdgeInsets.only(bottom: WsMetrics.space2),
          child: SingleChildScrollView(
            scrollDirection: Axis.horizontal,
            child: Row(
              children: [
                for (final (index, (kind, text)) in waiting.indexed)
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
                          onTap: () => state.unqueue(text),
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
    return CallbackShortcuts(
      bindings: {
        const SingleActivator(LogicalKeyboardKey.enter): () =>
            state.decide(BridgeDecision.allowOnce),
        const SingleActivator(LogicalKeyboardKey.enter, meta: true): () =>
            state.decide(BridgeDecision.allowSession),
        const SingleActivator(LogicalKeyboardKey.escape): () =>
            state.decide(BridgeDecision.reject),
      },
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
              Container(
                padding: const EdgeInsets.symmetric(
                  horizontal: WsMetrics.space4,
                  vertical: WsMetrics.space2,
                ),
                decoration: BoxDecoration(
                  color: ws.code,
                  borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                ),
                child: Text(
                  approval.arguments,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: monoStyle(context),
                ),
              ),
              const Spacer(),
              Row(
                children: [
                  PrimaryButton(
                    label: 'Allow once',
                    shortcut: '⏎',
                    onPressed: () => state.decide(BridgeDecision.allowOnce),
                  ),
                  const SizedBox(width: WsMetrics.space3),
                  SecondaryButton(
                    label: 'Allow for this session',
                    shortcut: '⌘⏎',
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
                    shortcut: 'esc',
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
    final answer = switch (question.kind) {
      BridgeQuestionKind.select => Wrap(
        spacing: WsMetrics.space3,
        children: [
          for (final option in question.options)
            SecondaryButton(
              label: option,
              onPressed: () => state.answerText(option),
            ),
        ],
      ),
      BridgeQuestionKind.confirm => Row(
        children: [
          PrimaryButton(
            label: 'Yes',
            shortcut: '⏎',
            onPressed: () => state.answerConfirm(true),
          ),
          const SizedBox(width: WsMetrics.space3),
          SecondaryButton(
            label: 'No',
            onPressed: () => state.answerConfirm(false),
          ),
        ],
      ),
      BridgeQuestionKind.input || BridgeQuestionKind.editor => Row(
        children: [
          Expanded(
            child: TextField(
              key: const Key('answer'),
              controller: _text,
              autofocus: true,
              decoration: InputDecoration.collapsed(hintText: question.detail),
              onSubmitted: (text) => state.answerText(text),
            ),
          ),
          PrimaryButton(
            label: 'Answer',
            shortcut: '⏎',
            onPressed: () => state.answerText(_text.text),
          ),
        ],
      ),
    };
    return CallbackShortcuts(
      bindings: {
        const SingleActivator(LogicalKeyboardKey.escape): state.dismiss,
        if (question.kind == BridgeQuestionKind.confirm)
          const SingleActivator(LogicalKeyboardKey.enter): () =>
              state.answerConfirm(true),
      },
      child: Focus(
        autofocus:
            question.kind != BridgeQuestionKind.input &&
            question.kind != BridgeQuestionKind.editor,
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
                    shortcut: 'esc',
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
              const Spacer(),
              answer,
            ],
          ),
        ),
      ),
    );
  }
}

/// A small keyboard hint such as ⌘L.
class Hint extends StatelessWidget {
  const Hint(this.text, {super.key, this.color});

  final String text;
  final Color? color;

  @override
  Widget build(BuildContext context) {
    final shade = (color ?? context.ws.textFaint).withValues(alpha: 0.7);
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space1),
      decoration: BoxDecoration(
        border: Border.all(color: shade),
        borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
      ),
      child: Text(
        text,
        style: TextStyle(fontSize: 11, height: 16 / 11, color: shade),
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
    return Material(
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
              if (shortcut != null) ...[
                const SizedBox(width: WsMetrics.space3),
                Hint(shortcut!, color: foreground),
              ],
            ],
          ),
        ),
      ),
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
