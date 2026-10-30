import 'package:flutter/material.dart';

import '../controls.dart';
import '../shortcuts/keymap.dart';
import '../rust/api/history.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'trajectory_model.dart';

/// The Trajectory's filter row (approved navigation mockup): kinds of step, the session's tool
/// names, failed steps only and text. Steps that do not match draw faint but keep their place.
class FilterBar extends StatelessWidget {
  const FilterBar({
    super.key,
    required this.filter,
    required this.tools,
    required this.text,
    required this.textFocus,
    required this.onChanged,
    required this.onDone,
  });

  final StepFilter filter;

  /// The tools the branch called, in order of first use.
  final List<String> tools;
  final TextEditingController text;
  final FocusNode textFocus;
  final ValueChanged<StepFilter> onChanged;

  /// Return or esc in the text field gives the keyboard back to the list.
  final VoidCallback onDone;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final faint = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
    Widget separator() => Padding(
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space1),
      child: Text('·', style: faint),
    );
    return Container(
      height: 36,
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space5),
      decoration: BoxDecoration(
        color: ws.subtle,
        border: Border(top: BorderSide(color: ws.border)),
      ),
      child: Row(
        children: [
          Text('Show', style: faint),
          const SizedBox(width: WsMetrics.space2),
          Expanded(
            child: SingleChildScrollView(
              scrollDirection: Axis.horizontal,
              child: Row(
                children: [
                  for (final (kind, label) in const [
                    (BridgeRecordKind.message, 'Model'),
                    (BridgeRecordKind.tool, 'Tools'),
                    (BridgeRecordKind.user, 'You'),
                    (BridgeRecordKind.approval, 'Approvals'),
                  ])
                    _Chip(
                      label: label,
                      on: filter.kinds.contains(kind),
                      onTap: () => onChanged(
                        filter.copyWith(kinds: _toggle(filter.kinds, kind)),
                      ),
                    ),
                  if (tools.isNotEmpty) separator(),
                  for (final tool in tools)
                    _Chip(
                      label: tool,
                      on: filter.tools.contains(tool),
                      onTap: () => onChanged(
                        filter.copyWith(tools: _toggle(filter.tools, tool)),
                      ),
                    ),
                  separator(),
                  _Chip(
                    label: 'Failed',
                    on: filter.failed,
                    onTap: () =>
                        onChanged(filter.copyWith(failed: !filter.failed)),
                  ),
                  const SizedBox(width: WsMetrics.space2),
                  SizedBox(
                    width: 180,
                    height: 26,
                    child: CallbackShortcuts(
                      bindings: {escapeKey: onDone},
                      child: TextField(
                        key: const Key('filter-text'),
                        onSubmitted: (_) => onDone(),
                        controller: text,
                        focusNode: textFocus,
                        onChanged: (value) =>
                            onChanged(filter.copyWith(text: value)),
                        style: const TextStyle(fontSize: WsMetrics.fsSmall),
                        decoration: InputDecoration(
                          isDense: true,
                          hintText: 'Filter steps…',
                          hintStyle: faint,
                          contentPadding: const EdgeInsets.symmetric(
                            horizontal: WsMetrics.space2,
                            vertical: 6,
                          ),
                          border: OutlineInputBorder(
                            borderRadius: BorderRadius.circular(
                              WsMetrics.radiusMd,
                            ),
                            borderSide: BorderSide(color: ws.borderStrong),
                          ),
                          enabledBorder: OutlineInputBorder(
                            borderRadius: BorderRadius.circular(
                              WsMetrics.radiusMd,
                            ),
                            borderSide: BorderSide(color: ws.borderStrong),
                          ),
                        ),
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ),
          const SizedBox(width: WsMetrics.space3),
          Shortcuts(
            shortcuts: activateKeys,
            child: InkWell(
              onTap: () {
                text.clear();
                onChanged(const StepFilter());
              },
              borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
              child: Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: WsMetrics.space2,
                  vertical: WsMetrics.space1,
                ),
                child: Text('Clear', style: faint),
              ),
            ),
          ),
        ],
      ),
    );
  }

  static Set<T> _toggle<T>(Set<T> set, T value) =>
      set.contains(value) ? ({...set}..remove(value)) : {...set, value};
}

/// A filter that is on or off; which one is told to assistive technology, not only drawn.
class _Chip extends StatelessWidget {
  const _Chip({required this.label, required this.on, required this.onTap});

  final String label;
  final bool on;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Padding(
      padding: const EdgeInsets.only(right: WsMetrics.space1),
      child: Semantics(
        selected: on,
        button: true,
        child: Shortcuts(
          shortcuts: activateKeys,
          child: InkWell(
            onTap: onTap,
            borderRadius: BorderRadius.circular(10),
            child: Container(
              height: 22,
              padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
              alignment: Alignment.center,
              decoration: BoxDecoration(
                color: on ? ws.accentSubtle : null,
                borderRadius: BorderRadius.circular(10),
                border: Border.all(
                  color: on
                      ? ws.accent.withValues(alpha: 0.4)
                      : ws.borderStrong,
                ),
              ),
              child: Text(
                label,
                style: TextStyle(
                  fontSize: WsMetrics.fsSmall,
                  color: on ? ws.accent : ws.textMuted,
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
