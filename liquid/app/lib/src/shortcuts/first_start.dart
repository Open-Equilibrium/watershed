import 'package:flutter/material.dart';

import '../controls.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'keymap.dart';

/// Asks once which profile to start with (approved mockup). Nothing is preselected;
/// esc keeps Standard. None when something else closed the question before a choice.
Future<Profile?> askProfile(BuildContext context) => showDialog<Profile>(
  context: context,
  barrierDismissible: false,
  builder: (context) => const _FirstStart(),
);

class _FirstStart extends StatelessWidget {
  const _FirstStart();

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    void choose(Profile profile) => Navigator.of(context).pop(profile);
    return CallbackShortcuts(
      bindings: {escapeKey: () => choose(Profile.standard)},
      // A focused node inside the dialog, so esc reaches the binding above.
      child: Focus(
        autofocus: true,
        child: Dialog(
          backgroundColor: ws.canvas,
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(WsMetrics.radiusXl),
          ),
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 620),
            child: Padding(
              padding: const EdgeInsets.all(WsMetrics.space6),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Text(
                    'How do you want to use the keyboard?',
                    style: Theme.of(context).textTheme.titleSmall
                        ?.copyWith(fontWeight: WsMetrics.fwSemibold),
                  ),
                  const SizedBox(height: WsMetrics.space2),
                  Text(
                    'Pick one. You can switch at any time in Settings → Shortcuts.',
                    style: TextStyle(color: ws.textMuted),
                  ),
                  const SizedBox(height: WsMetrics.space5),
                  IntrinsicHeight(
                    child: Row(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        Expanded(
                          child: _Choice(
                            cap: '⇪',
                            name: Profile.liquidKey.label,
                            points: const [
                              'Tap Caps Lock in any app to open Liquid.',
                              'In Liquid, hold it to see shortcuts; add a key to run one, like '
                                  'Caps Lock + N for a new session.',
                              'Shift + Caps Lock still turns capitals on or off.',
                              '⌘ shortcuts keep working.',
                            ],
                            note:
                                'Adds a small background item, so the key also opens Liquid when '
                                "it isn't running. macOS lists it under Login Items.",
                            action: 'Use ${Profile.liquidKey.label}',
                            onChoose: () => choose(Profile.liquidKey),
                          ),
                        ),
                        const SizedBox(width: WsMetrics.space4),
                        Expanded(
                          child: _Choice(
                            cap: '⌘',
                            name: Profile.standard.label,
                            points: const [
                              'Caps Lock stays as it is.',
                              '⌘ shortcuts, like ⌘N for a new session.',
                              'Hold ⌘ to see shortcuts.',
                            ],
                            action: 'Use Standard',
                            onChoose: () => choose(Profile.standard),
                          ),
                        ),
                      ],
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// One profile to choose; the whole card takes the click, Tab and Enter.
class _Choice extends StatelessWidget {
  const _Choice({
    required this.cap,
    required this.name,
    required this.points,
    required this.action,
    required this.onChoose,
    this.note,
  });

  final String cap;
  final String name;
  final List<String> points;
  final String? note;
  final String action;
  final VoidCallback onChoose;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Material(
      color: Colors.transparent,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(WsMetrics.radiusLg + 2),
        side: BorderSide(color: ws.borderStrong),
      ),
      child: InkWell(
        onTap: onChoose,
        borderRadius: BorderRadius.circular(WsMetrics.radiusLg + 2),
        child: Padding(
          padding: const EdgeInsets.all(WsMetrics.space4),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Row(
                children: [
                  Container(
                    width: 26,
                    height: 26,
                    alignment: Alignment.center,
                    decoration: BoxDecoration(
                      color: ws.subtle,
                      border: Border.all(color: ws.borderStrong),
                      borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                    ),
                    child: Text(cap),
                  ),
                  const SizedBox(width: WsMetrics.space3),
                  Text(
                    name,
                    style: const TextStyle(fontWeight: WsMetrics.fwSemibold),
                  ),
                ],
              ),
              const SizedBox(height: WsMetrics.space3),
              for (final point in points)
                Padding(
                  padding: const EdgeInsets.only(bottom: WsMetrics.space1),
                  child: Row(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      const Text('•  '),
                      Expanded(child: Text(point)),
                    ],
                  ),
                ),
              if (note case final note?) ...[
                const SizedBox(height: WsMetrics.space2),
                Text(
                  note,
                  style: TextStyle(
                    color: ws.textFaint,
                    fontSize: WsMetrics.fsSmall,
                  ),
                ),
              ],
              const Spacer(),
              const SizedBox(height: WsMetrics.space4),
              SecondaryButton(label: action, onPressed: onChoose),
            ],
          ),
        ),
      ),
    );
  }
}
