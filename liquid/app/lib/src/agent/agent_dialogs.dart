import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart'
    show Int64List;

import '../controls.dart';
import '../rust/api/details.dart';
import '../session/agents_state.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';

Future<void> showRenameAgent(BuildContext context, AgentsState agents) async {
  final name = agents.open.value?.name;
  if (name == null) return;
  final chosen = await showDialog<String>(
    context: context,
    builder: (_) => _TextDialog(
      title: 'Rename agent',
      action: 'Rename',
      initial: name,
      hint: 'Lowercase letters, digits and dashes.',
    ),
  );
  if (chosen != null && chosen != name) await agents.rename(chosen);
}

Future<void> showEditInstructions(
  BuildContext context,
  AgentsState agents,
) async {
  final text = await agents.instructions();
  if (text == null || !context.mounted) return;
  final saved = await showDialog<String>(
    context: context,
    builder: (_) => _TextDialog(
      title: 'Instructions · ${agents.open.value?.name ?? ''}',
      action: 'Save',
      initial: text,
      hint: 'The first line, # name, is the agent\'s name.',
      lines: 12,
    ),
  );
  if (saved != null && saved != text) await agents.setInstructions(saved);
}

class _TextDialog extends StatefulWidget {
  const _TextDialog({
    required this.title,
    required this.action,
    required this.initial,
    required this.hint,
    this.lines = 1,
  });

  final String title;
  final String action;
  final String initial;
  final String hint;
  final int lines;

  @override
  State<_TextDialog> createState() => _TextDialogState();
}

class _TextDialogState extends State<_TextDialog> {
  late final _text = TextEditingController(text: widget.initial);

  @override
  void dispose() {
    _text.dispose();
    super.dispose();
  }

  void _save() => Navigator.of(context).pop(_text.text);

  @override
  Widget build(BuildContext context) => ActionDialog(
    title: widget.title,
    bindings: {
      if (widget.lines == 1)
        const SingleActivator(LogicalKeyboardKey.enter): _save,
    },
    actions: [PrimaryButton(label: widget.action, onPressed: _save)],
    children: [
      TextField(
        key: const Key('agent-text'),
        controller: _text,
        autofocus: true,
        minLines: widget.lines,
        maxLines: widget.lines,
        decoration: const InputDecoration(isDense: true),
      ),
      Text(widget.hint, style: DangerDialog.text(context)),
    ],
  );
}

enum Runs { every, atTime, once, webhook }

const _days = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];

Future<void> showAddRoutine(BuildContext context, AgentsState agents) =>
    showDialog<void>(
      context: context,
      builder: (_) => AddRoutineDialog(agents: agents),
    );

class AddRoutineDialog extends StatefulWidget {
  const AddRoutineDialog({super.key, required this.agents});

  final AgentsState agents;

  @override
  State<AddRoutineDialog> createState() => _AddRoutineDialogState();
}

class _AddRoutineDialogState extends State<AddRoutineDialog> {
  var _runs = Runs.atTime;
  final _every = TextEditingController(text: '60');
  final _time = TextEditingController(text: '07:30');
  final _once = TextEditingController();
  final _title = TextEditingController();
  final _prompt = TextEditingController();
  final _weekdays = <int>{1, 2, 3, 4, 5};
  String? _problem;
  String? _webhook;
  var _saving = false;

  @override
  void dispose() {
    for (final field in [_every, _time, _once, _title, _prompt]) {
      field.dispose();
    }
    super.dispose();
  }

  BridgeSchedule? _schedule() {
    BridgeSchedule of(
      BridgeScheduleKind kind, {
      int? everyMs,
      String? time,
      List<int> weekdays = const [],
      String? runAt,
    }) => BridgeSchedule(
      kind: kind,
      everyMs: everyMs,
      timeOfDay: time,
      weekdays: Int64List.fromList(weekdays),
      runAt: runAt,
    );
    switch (_runs) {
      case Runs.every:
        final minutes = int.tryParse(_every.text.trim());
        if (minutes == null || minutes < 1) {
          _problem = 'Every is a number of minutes, at least 1.';
          return null;
        }
        return of(BridgeScheduleKind.interval, everyMs: minutes * 60000);
      case Runs.atTime:
        final time = _time.text.trim();
        if (!RegExp(r'^([01]\d|2[0-3]):[0-5]\d$').hasMatch(time)) {
          _problem = 'The time is HH:MM, such as 07:30.';
          return null;
        }
        return of(
          BridgeScheduleKind.fixedTime,
          time: time,
          weekdays: _weekdays.length == 7
              ? const []
              : (_weekdays.toList()..sort()),
        );
      case Runs.once:
        final at = DateTime.tryParse(_once.text.trim());
        if (at == null) {
          _problem = 'The time is a date and time, such as 2026-12-01 09:00.';
          return null;
        }
        return of(BridgeScheduleKind.once, runAt: _iso(at));
      case Runs.webhook:
        return of(BridgeScheduleKind.webhook);
    }
  }

  Future<void> _add() async {
    final prompt = _prompt.text.trim();
    setState(() => _problem = null);
    if (prompt.isEmpty) {
      setState(() => _problem = 'A routine needs a prompt.');
      return;
    }
    final schedule = _schedule();
    if (schedule == null) {
      setState(() {});
      return;
    }
    final title = _title.text.trim();
    setState(() => _saving = true);
    final added = await widget.agents.addRoutine(
      title.isEmpty ? prompt.split('\n').first : title,
      prompt,
      schedule,
    );
    if (!mounted) return;
    setState(() => _saving = false);
    if (added == null) return;
    if (added.webhook case final webhook?) {
      setState(() => _webhook = webhook);
    } else {
      Navigator.of(context).pop();
    }
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final name = widget.agents.open.value?.name ?? '';
    final label = TextStyle(color: ws.textMuted, fontSize: WsMetrics.fsCompact);
    Widget field(String text, Widget child) => Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        SizedBox(
          width: 72,
          child: Padding(
            padding: const EdgeInsets.only(top: WsMetrics.space2),
            child: Text(text, style: label),
          ),
        ),
        Expanded(child: child),
      ],
    );
    if (_webhook case final webhook?) {
      return ActionDialog(
        title: 'Add routine · $name',
        dismiss: 'Done',
        actions: const [],
        children: [
          field(
            'URL',
            Row(
              children: [
                Expanded(
                  child: SelectableText(
                    webhook,
                    key: const Key('routine-webhook'),
                    style: monoStyle(context),
                  ),
                ),
                GhostButton(
                  label: 'Copy',
                  onPressed: () => unawaited(
                    Clipboard.setData(ClipboardData(text: webhook)),
                  ),
                ),
              ],
            ),
          ),
          Text(
            'It reaches only this agent host.',
            style: DangerDialog.text(context),
          ),
        ],
      );
    }
    return ActionDialog(
      title: 'Add routine · $name',
      actions: [
        PrimaryButton(
          key: const Key('routine-add'),
          label: 'Add routine',
          onPressed: _saving ? null : () => unawaited(_add()),
        ),
      ],
      children: [
        field(
          'Runs',
          Align(
            alignment: Alignment.centerLeft,
            child: Segments<Runs>(
              options: const [
                (Runs.every, 'Every'),
                (Runs.atTime, 'At a time'),
                (Runs.once, 'Once'),
                (Runs.webhook, 'Webhook'),
              ],
              selected: _runs,
              dense: true,
              onSelect: (runs) => setState(() => _runs = runs),
            ),
          ),
        ),
        switch (_runs) {
          Runs.every => field(
            'Minutes',
            TextField(
              controller: _every,
              decoration: const InputDecoration(isDense: true),
            ),
          ),
          Runs.atTime => field(
            'Time',
            Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              spacing: WsMetrics.space2,
              children: [
                SizedBox(
                  width: 96,
                  child: TextField(
                    controller: _time,
                    decoration: const InputDecoration(isDense: true),
                  ),
                ),
                Wrap(
                  spacing: WsMetrics.space1,
                  children: [
                    for (final (index, day) in _days.indexed)
                      FilterChip(
                        label: Text(day),
                        selected: _weekdays.contains((index + 1) % 7),
                        showCheckmark: false,
                        visualDensity: VisualDensity.compact,
                        onSelected: (on) => setState(() {
                          final weekday = (index + 1) % 7;
                          if (on) {
                            _weekdays.add(weekday);
                          } else if (_weekdays.length > 1) {
                            _weekdays.remove(weekday);
                          }
                        }),
                      ),
                  ],
                ),
              ],
            ),
          ),
          Runs.once => field(
            'At',
            TextField(
              controller: _once,
              decoration: const InputDecoration(
                isDense: true,
                hintText: '2026-12-01 09:00',
              ),
            ),
          ),
          Runs.webhook => Text(
            'The prompt can use {{body.path}}, {{headers.name}}, '
            '{{query.name}} and {{body}}.',
            style: DangerDialog.text(context),
          ),
        },
        field(
          'Title',
          TextField(
            controller: _title,
            decoration: const InputDecoration(isDense: true),
          ),
        ),
        field(
          'Prompt',
          TextField(
            key: const Key('routine-prompt'),
            controller: _prompt,
            minLines: 3,
            maxLines: 6,
            decoration: const InputDecoration(isDense: true),
          ),
        ),
        if (_problem case final problem?)
          Text(problem, style: TextStyle(color: ws.danger)),
      ],
    );
  }
}

String _iso(DateTime local) {
  final offset = local.timeZoneOffset;
  final sign = offset.isNegative ? '-' : '+';
  final minutes = offset.inMinutes.abs();
  String two(int value) => value.toString().padLeft(2, '0');
  return '${local.year.toString().padLeft(4, '0')}-${two(local.month)}-'
      '${two(local.day)}T${two(local.hour)}:${two(local.minute)}:00'
      '$sign${two(minutes ~/ 60)}:${two(minutes % 60)}';
}
