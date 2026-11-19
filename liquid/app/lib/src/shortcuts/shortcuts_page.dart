import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../controls.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'keymap.dart';
import 'shortcut_store.dart';

class _Entry {
  const _Entry(Command this.command) : fixed = null, system = null;
  const _Entry.fixed(Fixed this.fixed) : command = null, system = null;

  const _Entry.system(String label, String keys)
    : system = (label, keys),
      command = null,
      fixed = null;

  final Command? command;
  final Fixed? fixed;
  final (String, String)? system;

  String? get fixedLabel => fixed?.label ?? system?.$1;
  String? get fixedKeys => fixed?.display ?? system?.$2;
}

typedef _Group = (String title, String? note, List<_Entry> entries);

const _liquidKeyGroup = (
  'Liquid key',
  'every app',
  [
    _Entry.system('Open Liquid', 'tap ⇪'),
    _Entry.system('Caps Lock on or off', '⇧⇪'),
    _Entry.system('Show shortcuts in Liquid', 'hold ⇪'),
  ],
);

const List<_Group> _left = [
  (
    'App',
    'menu bar shows ⌘',
    [
      _Entry(Command.newSession),
      _Entry(Command.newPage),
      _Entry(Command.settings),
      _Entry(Command.chat),
      _Entry(Command.diff),
      _Entry(Command.sessionDetails),
      _Entry(Command.statistics),
      _Entry(Command.focusInput),
      _Entry.fixed(Fixed.switchWorkspace),
    ],
  ),
  (
    'Input area',
    null,
    [
      _Entry(Command.send),
      _Entry(Command.queue),
      _Entry.fixed(Fixed.newLine),
      _Entry(Command.stop),
      _Entry(Command.dismissNotice),
    ],
  ),
  (
    'Page',
    null,
    [_Entry(Command.toggleToDo), _Entry.fixed(Fixed.newBlockOrLine)],
  ),
];

const List<_Group> _right = [
  (
    'Agent questions',
    null,
    [
      _Entry(Command.answer),
      _Entry(Command.answerEditor),
      _Entry.fixed(Fixed.moveThroughOptions),
      _Entry(Command.dismissQuestion),
    ],
  ),
  (
    'Diff',
    null,
    [
      _Entry.fixed(Fixed.previousOrNextFile),
      _Entry.fixed(Fixed.foldFolder),
      _Entry.fixed(Fixed.previousOrNextHunk),
      _Entry.fixed(Fixed.unifiedOrSplit),
      _Entry.fixed(Fixed.hideWhitespace),
      _Entry.fixed(Fixed.turnMenu),
    ],
  ),
];

class ShortcutsPane extends StatefulWidget {
  const ShortcutsPane({super.key, required this.shortcuts});

  final ShortcutController shortcuts;

  @override
  State<ShortcutsPane> createState() => _ShortcutsPaneState();
}

class _ShortcutsPaneState extends State<ShortcutsPane> {
  final _keys = FocusNode(debugLabel: 'shortcuts');
  final _search = TextEditingController();

  Command? _recording;
  Keys? _pressed;
  List<({Keys keys, List<Command> owners})> _conflicts = const [];

  bool _restoring = false;

  String? _refused;

  ({Command command, Keys keys, List<Command> clear, Profile profile})?
  _replacing;
  final _keep = FocusNode(debugLabel: 'keep Custom');
  final _replace = FocusNode(debugLabel: 'replace Custom');

  ShortcutController get _shortcuts => widget.shortcuts;

  @override
  void initState() {
    super.initState();
    _search.addListener(() => setState(() {}));
    _keys.addListener(_focusChanged);
    _shortcuts.addListener(_settingsChanged);
  }

  @override
  void dispose() {
    _shortcuts.removeListener(_settingsChanged);
    _keys.dispose();
    _keep.dispose();
    _replace.dispose();
    _search.dispose();
    super.dispose();
  }

  void _record(Command command) {
    setState(() {
      _recording = command;
      _pressed = null;
      _conflicts = const [];
      _refused = null;
      _restoring = false;
    });
    _keys.requestFocus();
  }

  void _stop() => setState(() {
    _recording = null;
    _pressed = null;
    _conflicts = const [];
    _refused = null;
    _replacing = null;
    _restoring = false;
  });

  void _reset(Command command) {
    final restored = _shortcuts.value.reset(command);
    final all = restored.withLiquidKey(true);
    final conflicts = [
      for (final keys in all.bindings(command))
        if (all.conflicts(command, keys) case final owners
            when owners.isNotEmpty)
          (keys: keys, owners: owners),
    ];
    if (conflicts.isEmpty) {
      _shortcuts.change(restored);
      return;
    }
    setState(() {
      _recording = command;
      _pressed = conflicts.first.keys;
      _conflicts = conflicts;
      _refused = null;
      _restoring = true;
    });
    _keys.requestFocus();
  }

  void _restore(Command command) {
    _shortcuts.change(_shortcuts.value.resetTaking(command));
    _done();
  }

  void _focusChanged() {
    if (_recording != null &&
        !_keys.hasPrimaryFocus &&
        _conflicts.isEmpty &&
        _replacing == null) {
      _stop();
    }
  }

  void _settingsChanged() {
    final replacing = _replacing;
    if (replacing != null &&
        (_shortcuts.value.replaced == null ||
            _shortcuts.value.profile != replacing.profile)) {
      _done();
    }
    if (_conflicts.isNotEmpty) _stop();
  }

  void _change(Command command, Keys keys, {List<Command> clear = const []}) {
    if (_shortcuts.value.replaced == null) return _apply(command, keys, clear);
    setState(() {
      _pressed = keys;
      _conflicts = const [];
      _refused = null;
      _replacing = (
        command: command,
        keys: keys,
        clear: clear,
        profile: _shortcuts.value.profile,
      );
    });
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _keep.requestFocus();
    });
  }

  void _apply(Command command, Keys keys, List<Command> clear) {
    _shortcuts.change(_shortcuts.value.change(command, keys, clear: clear));
    _done();
  }

  void _done() {
    _stop();
    _keys.requestFocus();
  }

  static final _modifiers = {
    LogicalKeyboardKey.metaLeft,
    LogicalKeyboardKey.metaRight,
    LogicalKeyboardKey.altLeft,
    LogicalKeyboardKey.altRight,
    LogicalKeyboardKey.shiftLeft,
    LogicalKeyboardKey.shiftRight,
    LogicalKeyboardKey.controlLeft,
    LogicalKeyboardKey.controlRight,
    LogicalKeyboardKey.capsLock,
    LogicalKeyboardKey.fn,
    liquidKey,
  };

  static final _navigation = {LogicalKeyboardKey.tab};

  KeyEventResult _onKey(FocusNode node, KeyEvent event) {
    final command = _recording;
    if (command == null ||
        !node.hasPrimaryFocus ||
        _navigation.contains(event.logicalKey)) {
      return KeyEventResult.ignored;
    }
    if (event is! KeyDownEvent || _modifiers.contains(event.logicalKey)) {
      return KeyEventResult.handled;
    }
    final keys = Keys.pressed(event.logicalKey, HardwareKeyboard.instance);
    if (keys == const Keys(LogicalKeyboardKey.escape)) {
      _stop();
      return KeyEventResult.handled;
    }
    if (_shortcuts.value.bindings(command).firstOrNull == keys) {
      _done();
      return KeyEventResult.handled;
    }
    final use = reserved(command, keys);
    final owners = _shortcuts.value
        .withLiquidKey(true)
        .conflicts(command, keys);
    if (use == null && owners.isEmpty) {
      _change(command, keys);
    } else {
      setState(() {
        _pressed = keys;
        _conflicts = use == null && owners.isNotEmpty
            ? [(keys: keys, owners: owners)]
            : const [];
        _restoring = false;
        _refused = switch (use) {
          null => null,
          'typing' => '${keys.label} is for typing here.',
          _ => '${keys.label} is kept for $use.',
        };
      });
    }
    return KeyEventResult.handled;
  }

  bool _matches(_Entry entry, ShortcutSettings settings) {
    final query = _search.text.trim().toLowerCase();
    if (query.isEmpty) return true;
    final command = entry.command;
    final texts = command == null
        ? [entry.fixedLabel!, entry.fixedKeys!]
        : [
            command.label,
            for (final keys in settings.bindings(command)) keys.label,
          ];
    return texts.any((text) => text.toLowerCase().contains(query));
  }

  @override
  Widget build(BuildContext context) => Focus(
    focusNode: _keys,
    onKeyEvent: _onKey,
    child: ValueListenableBuilder(valueListenable: _shortcuts, builder: _main),
  );

  Widget _main(BuildContext context, ShortcutSettings settings, Widget? _) {
    final ws = context.ws;
    final small = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
    List<_Group> visible(List<_Group> groups) => [
      for (final (title, note, entries) in groups)
        if (entries.where((entry) => _matches(entry, settings)).toList()
            case final shown when shown.isNotEmpty)
          (title, note, shown),
    ];
    final left = visible([if (settings.liquidKey) _liquidKeyGroup, ..._left]);
    final right = visible(_right);
    final changed = settings.changes.length;
    return SingleChildScrollView(
      padding: const EdgeInsets.fromLTRB(
        WsMetrics.space6,
        WsMetrics.space5,
        WsMetrics.space6,
        WsMetrics.space5,
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              Text(
                'Shortcuts',
                style: Theme.of(context).textTheme.titleMedium
                    ?.copyWith(fontWeight: WsMetrics.fwSemibold),
              ),
              const SizedBox(width: WsMetrics.space4),
              Expanded(
                child: Container(
                  height: 28,
                  padding: const EdgeInsets.symmetric(
                    horizontal: WsMetrics.space3,
                  ),
                  alignment: Alignment.centerLeft,
                  decoration: BoxDecoration(
                    border: Border.all(color: ws.borderStrong),
                    borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                  ),
                  child: TextField(
                    controller: _search,
                    style: const TextStyle(fontSize: WsMetrics.fsCompact),
                    decoration: InputDecoration.collapsed(
                      hintText: 'Search actions or keys',
                      hintStyle: TextStyle(color: ws.textFaint),
                    ),
                  ),
                ),
              ),
            ],
          ),
          const SizedBox(height: WsMetrics.space3),
          _ProfileRow(shortcuts: _shortcuts, settings: settings),
          if (settings.liquidKey)
            Padding(
              padding: const EdgeInsets.fromLTRB(
                WsMetrics.space3,
                0,
                WsMetrics.space3,
                WsMetrics.space2,
              ),
              child: Text(
                'Caps Lock is the Liquid key. Liquid commands use it; their ⌘ shortcuts keep '
                'working and show in the menu bar.',
                style: TextStyle(
                  color: ws.textMuted,
                  fontSize: WsMetrics.fsSmall,
                ),
              ),
            ),
          LayoutBuilder(
            builder: (context, constraints) {
              Widget column(List<_Group> groups) => Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  for (final group in groups) _group(context, settings, group),
                ],
              );
              if (constraints.maxWidth < 640) {
                return column([...left, ...right]);
              }
              return Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Expanded(child: column(left)),
                  const SizedBox(width: WsMetrics.space7),
                  Expanded(child: column(right)),
                ],
              );
            },
          ),
          const SizedBox(height: WsMetrics.space4),
          Container(
            padding: const EdgeInsets.only(top: WsMetrics.space3),
            decoration: BoxDecoration(
              border: Border(top: BorderSide(color: ws.border)),
            ),
            child: Row(
              children: [
                Text(
                  changed > 0
                      ? '$changed shortcut${changed == 1 ? '' : 's'} changed'
                      : '${settings.profile.label} profile',
                  style: small,
                ),
                const Spacer(),
                _Link(
                  'Reset all to defaults',
                  onTap: () => _shortcuts.change(settings.resetAll()),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  Widget _group(BuildContext context, ShortcutSettings settings, _Group group) {
    final ws = context.ws;
    final (title, note, entries) = group;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(
            WsMetrics.space3,
            WsMetrics.space3,
            WsMetrics.space3,
            WsMetrics.space1,
          ),
          child: Text.rich(
            TextSpan(
              text: title,
              children: [
                if (note != null)
                  TextSpan(
                    text: '  $note',
                    style: TextStyle(
                      color: ws.textFaint,
                      fontWeight: WsMetrics.fwRegular,
                    ),
                  ),
              ],
            ),
            style: TextStyle(
              color: ws.textMuted,
              fontSize: WsMetrics.fsSmall,
              fontWeight: WsMetrics.fwSemibold,
            ),
          ),
        ),
        for (final entry in entries)
          if (entry.command case final command?)
            ..._commandRows(context, settings, command)
          else
            _Row(label: entry.fixedLabel!, keys: [_Kbd(entry.fixedKeys!)]),
      ],
    );
  }

  List<Widget> _commandRows(
    BuildContext context,
    ShortcutSettings settings,
    Command command,
  ) {
    final bindings = settings.bindings(command);
    if (_recording == command) {
      final pressed = _pressed;
      final conflicts = _conflicts;
      final owners = {for (final conflict in conflicts) ...conflict.owners}
          .toList();
      final refused = _refused;
      return [
        _Row(
          label: command.label,
          recording: true,
          keys: [
            _Kbd(pressed?.label ?? 'Press keys…', tone: _Tone.recording),
            _Link('Cancel', onTap: _stop),
          ],
        ),
        if (refused != null)
          _Notice(children: [Text(refused, style: _Notice.text(context))]),
        if (_replacing case final replacing? when settings.replaced != null)
          _ReplaceCustom(
            settings: settings,
            change: replacing,
            keep: _keep,
            replace: _replace,
            onKeep: _done,
            onReplace: () =>
                _apply(replacing.command, replacing.keys, replacing.clear),
          ),
        if (pressed != null && conflicts.isNotEmpty)
          _Notice(
            children: [
              for (final conflict in conflicts)
                Text.rich(
                  TextSpan(
                    text: '${conflict.keys.label} already runs ',
                    children: _ownerSpans(conflict.owners),
                  ),
                  style: _Notice.text(context),
                ),
              _Link(
                'Use anyway, clear ${_ownerNames(owners)}',
                onTap: () => _restoring
                    ? _restore(command)
                    : _change(command, pressed, clear: owners),
              ),
            ],
          ),
      ];
    }
    final changed = settings.changed(command);
    final defaults = settings.defaultLabel(command);
    return [
      _Row(
        label: command.label,
        onTap: () => _record(command),
        keys: [
          if (bindings.isEmpty) _Kbd('—', tone: _Tone.faint),
          for (final (index, keys) in bindings.indexed)
            _Kbd(
              keys.label,
              tone: index > 0
                  ? _Tone.faint
                  : changed
                  ? _Tone.changed
                  : _Tone.plain,
            ),
          if (changed)
            _Link(
              defaults.isEmpty ? 'Reset' : 'Reset to $defaults',
              onTap: () => _reset(command),
            ),
        ],
      ),
    ];
  }
}

class _ProfileRow extends StatelessWidget {
  const _ProfileRow({required this.shortcuts, required this.settings});

  final ShortcutController shortcuts;
  final ShortcutSettings settings;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final available = shortcuts.liquidKeyAvailable;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.symmetric(
            horizontal: WsMetrics.space3,
            vertical: WsMetrics.space2,
          ),
          child: Wrap(
            spacing: WsMetrics.space4,
            crossAxisAlignment: WrapCrossAlignment.center,
            children: [
              Text('Profile', style: TextStyle(color: ws.textMuted)),
              Segments(
                options: [
                  for (final profile in Profile.values)
                    if (available || profile != Profile.liquidKey)
                      (profile, profile.label),
                ],
                selected: settings.profile,
                onSelect: (profile) =>
                    shortcuts.change(settings.choose(profile)),
              ),
              if (settings.profile == Profile.custom)
                Text(
                  'based on ${settings.base.label}',
                  style: TextStyle(
                    color: ws.textFaint,
                    fontSize: WsMetrics.fsSmall,
                  ),
                ),
            ],
          ),
        ),
        ValueListenableBuilder(
          valueListenable: shortcuts.liquidKeyProblem,
          builder: (context, problem, _) => problem == null
              ? const SizedBox.shrink()
              : _Notice(
                  children: [Text(problem, style: _Notice.text(context))],
                ),
        ),
        if (available && settings.profile == Profile.custom)
          _Row(
            label: 'Use Caps Lock as the Liquid key',
            keys: [
              SizedBox(
                height: 20,
                child: FittedBox(
                  child: Switch(
                    value: settings.liquidKey,
                    onChanged: (on) =>
                        shortcuts.change(settings.withLiquidKey(on)),
                  ),
                ),
              ),
            ],
          ),
      ],
    );
  }
}

class _Row extends StatelessWidget {
  const _Row({
    required this.label,
    required this.keys,
    this.onTap,
    this.recording = false,
  });

  final String label;
  final List<Widget> keys;
  final VoidCallback? onTap;
  final bool recording;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Material(
      color: recording ? ws.accentSubtle : Colors.transparent,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        side: recording ? BorderSide(color: ws.accent) : BorderSide.none,
      ),
      child: InkWell(
        onTap: onTap,
        canRequestFocus: onTap != null,
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        child: Container(
          height: WsMetrics.rowNav,
          padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
          child: Row(
            children: [
              Expanded(
                child: Text(
                  label,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(fontSize: WsMetrics.fsCompact),
                ),
              ),
              for (final key in keys) ...[
                const SizedBox(width: WsMetrics.space2),
                key,
              ],
            ],
          ),
        ),
      ),
    );
  }
}

enum _Tone { plain, faint, changed, recording }

List<InlineSpan> _ownerSpans(List<Command> owners) => [
  for (final (index, owner) in owners.indexed) ...[
    if (index > 0) TextSpan(text: index == owners.length - 1 ? ' and ' : ', '),
    TextSpan(
      text: owner.label,
      style: const TextStyle(fontWeight: WsMetrics.fwSemibold),
    ),
  ],
  const TextSpan(text: '.'),
];

String _ownerNames(List<Command> owners) {
  final labels = [for (final owner in owners) owner.label];
  return labels.length < 2
      ? labels.join()
      : '${labels.sublist(0, labels.length - 1).join(', ')} and ${labels.last}';
}

class _Kbd extends StatelessWidget {
  const _Kbd(this.text, {this.tone = _Tone.plain});

  final String text;
  final _Tone tone;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final border = switch (tone) {
      _Tone.plain || _Tone.faint => ws.borderStrong,
      _Tone.changed || _Tone.recording => ws.accent,
    };
    return Container(
      constraints: const BoxConstraints(minWidth: 22),
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space2),
      alignment: Alignment.center,
      decoration: BoxDecoration(
        color: switch (tone) {
          _Tone.plain || _Tone.changed => ws.subtle,
          _Tone.faint => null,
          _Tone.recording => ws.canvas,
        },
        border: Border.all(color: border),
        borderRadius: BorderRadius.circular(5),
      ),
      child: Text(
        text,
        style: TextStyle(
          fontSize: WsMetrics.fsSmall,
          height: 20 / WsMetrics.fsSmall,
          fontWeight: WsMetrics.fwMedium,
          color: switch (tone) {
            _Tone.plain || _Tone.changed => ws.text,
            _Tone.faint => ws.textFaint,
            _Tone.recording => ws.accent,
          },
        ),
      ),
    );
  }
}

class _Notice extends StatelessWidget {
  const _Notice({required this.children});

  final List<Widget> children;

  static TextStyle text(BuildContext context) =>
      TextStyle(color: context.ws.warning, fontSize: WsMetrics.fsSmall);

  @override
  Widget build(BuildContext context) => Container(
    margin: const EdgeInsets.symmetric(vertical: WsMetrics.space1),
    padding: const EdgeInsets.symmetric(
      horizontal: WsMetrics.space3,
      vertical: WsMetrics.space2,
    ),
    decoration: BoxDecoration(
      color: context.ws.warningSubtle,
      borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
    ),
    child: Material(
      type: MaterialType.transparency,
      child: Wrap(spacing: WsMetrics.space3, children: children),
    ),
  );
}

class _Link extends StatelessWidget {
  const _Link(this.text, {required this.onTap});

  final String text;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) => InkWell(
    onTap: onTap,
    borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
    child: Text(
      text,
      style: TextStyle(color: context.ws.accent, fontSize: WsMetrics.fsSmall),
    ),
  );
}

class _ReplaceCustom extends StatelessWidget {
  const _ReplaceCustom({
    required this.settings,
    required this.change,
    required this.keep,
    required this.replace,
    required this.onKeep,
    required this.onReplace,
  });

  final ShortcutSettings settings;
  final ({Command command, Keys keys, List<Command> clear, Profile profile})
  change;
  final FocusNode keep;
  final FocusNode replace;
  final VoidCallback onKeep;
  final VoidCallback onReplace;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final kept = settings.replaced!;
    final changed = kept.changes.length;
    final lost = changed > 0
        ? '$changed changed shortcut${changed == 1 ? '' : 's'}'
        : 'the Liquid key turned ${kept.liquidKey ? 'on' : 'off'}';
    return CallbackShortcuts(
      bindings: {escapeKey: onKeep},
      child: Focus(
        canRequestFocus: false,
        skipTraversal: true,
        onFocusChange: (inside) {
          if (!inside) onKeep();
        },
        child: Container(
          margin: const EdgeInsets.symmetric(vertical: WsMetrics.space1),
          padding: const EdgeInsets.symmetric(
            horizontal: WsMetrics.space4,
            vertical: WsMetrics.space3,
          ),
          decoration: BoxDecoration(
            color: ws.warningSubtle,
            border: Border.all(color: ws.warningBorder),
            borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
          ),
          child: Material(
            type: MaterialType.transparency,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Row(
                  children: [
                    Icon(
                      LucideIcons.triangleAlert,
                      size: 14,
                      color: ws.warning,
                    ),
                    const SizedBox(width: WsMetrics.space3),
                    const Text(
                      'Replace the Custom profile?',
                      style: TextStyle(fontWeight: WsMetrics.fwSemibold),
                    ),
                  ],
                ),
                const SizedBox(height: WsMetrics.space1),
                Text(
                  'Changing ${change.command.label} to ${change.keys.label} starts a '
                  'new Custom profile from ${settings.profile.label}. This '
                  'replaces your current Custom profile, which has $lost.',
                ),
                Text(
                  'To change that profile instead, cancel and choose Custom.',
                  style: TextStyle(
                    color: ws.textMuted,
                    fontSize: WsMetrics.fsSmall,
                  ),
                ),
                const SizedBox(height: WsMetrics.space3),
                FocusTraversalGroup(
                  policy: OrderedTraversalPolicy(),
                  child: Row(
                    mainAxisAlignment: MainAxisAlignment.end,
                    children: [
                      FocusTraversalOrder(
                        order: const NumericFocusOrder(2),
                        child: SecondaryButton(
                          label: 'Replace Custom',
                          color: ws.danger,
                          focusNode: replace,
                          onPressed: onReplace,
                        ),
                      ),
                      const SizedBox(width: WsMetrics.space3),
                      FocusTraversalOrder(
                        order: const NumericFocusOrder(1),
                        child: PrimaryButton(
                          label: 'Cancel',
                          shortcut: '⏎',
                          focusNode: keep,
                          onPressed: onKeep,
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
    );
  }
}
