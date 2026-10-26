import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../controls.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'keymap.dart';
import 'shortcut_store.dart';

/// One line of the list: a command whose keys can change, or keys that stay as they are.
class _Entry {
  const _Entry(Command this.command) : fixed = null, system = null;
  const _Entry.fixed(Fixed this.fixed) : command = null, system = null;

  /// Keys macOS handles for the Liquid key, which Liquid cannot change.
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
      _Entry(Command.settings),
      _Entry(Command.chat),
      _Entry(Command.trajectory),
      _Entry(Command.diff),
      _Entry(Command.focusInput),
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
];

const List<_Group> _right = [
  (
    'Approvals',
    null,
    [
      _Entry(Command.allowOnce),
      _Entry(Command.allowSession),
      _Entry(Command.reject),
      _Entry.fixed(Fixed.scrollCall),
    ],
  ),
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
    'Trajectory',
    null,
    [
      _Entry.fixed(Fixed.previousOrNextStep),
      _Entry.fixed(Fixed.foldTurn),
      _Entry(Command.foldAll),
      _Entry(Command.openStep),
      _Entry(Command.closeStep),
      _Entry(Command.branch),
      _Entry(Command.compare),
      _Entry.fixed(Fixed.moveInTree),
      _Entry(Command.selectBranch),
    ],
  ),
  (
    'Diff',
    null,
    [_Entry.fixed(Fixed.previousOrNextFile), _Entry.fixed(Fixed.foldFolder)],
  ),
];

const _profileNames = {
  Profile.liquidKey: 'Liquid key',
  Profile.standard: 'Standard',
  Profile.custom: 'Custom',
};

/// Settings → Shortcuts (approved mockup, ADR-0204, ADR-0205): the profile, every action's
/// keys and recording new ones. It replaces the main area; esc closes it.
class ShortcutsPage extends StatefulWidget {
  const ShortcutsPage({
    super.key,
    required this.shortcuts,
    required this.onClose,
  });

  final ShortcutController shortcuts;
  final VoidCallback onClose;

  @override
  State<ShortcutsPage> createState() => _ShortcutsPageState();
}

class _ShortcutsPageState extends State<ShortcutsPage> {
  final _keys = FocusNode(debugLabel: 'shortcuts');
  final _search = TextEditingController();

  /// The command taking new keys, the keys pressed for it and who runs them already.
  Command? _recording;
  Keys? _pressed;
  Command? _conflict;

  /// Why the keys pressed cannot be taken.
  String? _refused;

  /// A change waiting until the user agrees that it replaces the kept Custom profile.
  ({Command command, Keys keys, Command? clear})? _replacing;
  final _keep = FocusNode(debugLabel: 'keep Custom');
  final _replace = FocusNode(debugLabel: 'replace Custom');

  ShortcutController get _shortcuts => widget.shortcuts;

  @override
  void initState() {
    super.initState();
    _search.addListener(() => setState(() {}));
    _keys.addListener(_focusChanged);
    // The shell keeps the focus otherwise, and esc would not reach the page.
    WidgetsBinding.instance.addPostFrameCallback((_) => _keys.requestFocus());
  }

  @override
  void dispose() {
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
      _conflict = null;
      _refused = null;
    });
    _keys.requestFocus();
  }

  void _stop() => setState(() {
    _recording = null;
    _pressed = null;
    _conflict = null;
    _refused = null;
    _replacing = null;
  });

  /// Leaving the recording row stops recording, unless the focus went on to "Use anyway" or
  /// to the question whether to replace Custom.
  void _focusChanged() {
    if (_recording != null &&
        !_keys.hasPrimaryFocus &&
        _conflict == null &&
        _replacing == null) {
      _stop();
    }
  }

  /// Changes `command`'s keys, after asking when that replaces the kept Custom profile.
  void _change(Command command, Keys keys, {Command? clear}) {
    if (_shortcuts.value.replaced == null) return _apply(command, keys, clear);
    setState(() {
      _pressed = keys;
      _conflict = null;
      _replacing = (command: command, keys: keys, clear: clear);
    });
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _keep.requestFocus();
    });
  }

  void _apply(Command command, Keys keys, Command? clear) {
    _shortcuts.update(_shortcuts.value.change(command, keys, clear: clear));
    _done();
  }

  /// Ends recording and gives the keys back to the page.
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

  /// Tab still moves the focus, so "Use anyway" can be reached from the keyboard.
  static final _navigation = {LogicalKeyboardKey.tab};

  /// While the recording row has the focus, its keys go to the recorder, so no shortcut runs
  /// meanwhile; keys for the search field or a link pass.
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
    final use = reserved(command, keys);
    final conflict = _shortcuts.value.conflict(command, keys);
    if (use == null && conflict == null) {
      _change(command, keys);
    } else {
      setState(() {
        _pressed = keys;
        _conflict = use == null ? conflict : null;
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
  Widget build(BuildContext context) {
    final ws = context.ws;
    return CallbackShortcuts(
      bindings: {
        const SingleActivator(LogicalKeyboardKey.escape): widget.onClose,
      },
      child: Focus(
        focusNode: _keys,
        onKeyEvent: _onKey,
        child: ValueListenableBuilder(
          valueListenable: _shortcuts,
          builder: (context, settings, _) => Row(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Container(
                width: 150,
                padding: const EdgeInsets.all(WsMetrics.space4),
                decoration: BoxDecoration(
                  color: ws.subtle,
                  border: Border(right: BorderSide(color: ws.border)),
                ),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    Text(
                      'Settings',
                      style: TextStyle(
                        color: ws.textFaint,
                        fontSize: WsMetrics.fsSmall,
                      ),
                    ),
                    const SizedBox(height: WsMetrics.space2),
                    Container(
                      height: WsMetrics.rowNav,
                      padding: const EdgeInsets.symmetric(
                        horizontal: WsMetrics.space3,
                      ),
                      alignment: Alignment.centerLeft,
                      decoration: BoxDecoration(
                        color: ws.selected,
                        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                      ),
                      child: const Text(
                        'Shortcuts',
                        style: TextStyle(fontWeight: WsMetrics.fwMedium),
                      ),
                    ),
                  ],
                ),
              ),
              Expanded(child: _main(context, settings)),
            ],
          ),
        ),
      ),
    );
  }

  Widget _main(BuildContext context, ShortcutSettings settings) {
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
                      : '${_profileNames[settings.profile]} profile',
                  style: small,
                ),
                const Spacer(),
                _Link(
                  'Reset all to defaults',
                  onTap: () => _shortcuts.update(settings.resetAll()),
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
      final conflict = _conflict;
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
        if (_replacing case final replacing?)
          _ReplaceCustom(
            settings: settings,
            change: replacing,
            keep: _keep,
            replace: _replace,
            onKeep: _done,
            onReplace: () =>
                _apply(replacing.command, replacing.keys, replacing.clear),
          ),
        if (pressed != null && conflict != null)
          _Notice(
            children: [
              Text.rich(
                TextSpan(
                  text: '${pressed.label} already runs ',
                  children: [
                    TextSpan(
                      text: conflict.label,
                      style: const TextStyle(fontWeight: WsMetrics.fwSemibold),
                    ),
                    const TextSpan(text: '.'),
                  ],
                ),
                style: _Notice.text(context),
              ),
              _Link(
                'Use anyway, clear ${conflict.label}',
                onTap: () => _change(command, pressed, clear: conflict),
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
              onTap: () => _shortcuts.update(settings.reset(command)),
            ),
        ],
      ),
    ];
  }
}

/// The profile choice, and in Custom whether Caps Lock is the Liquid key.
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
              Container(
                padding: const EdgeInsets.all(2),
                decoration: BoxDecoration(
                  color: ws.subtle,
                  border: Border.all(color: ws.borderStrong),
                  borderRadius: BorderRadius.circular(WsMetrics.radiusMd + 1),
                ),
                // Focus shows above the group's background.
                child: Material(
                  type: MaterialType.transparency,
                  child: Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      for (final profile in Profile.values)
                        if (available || profile != Profile.liquidKey)
                          _Segment(
                            label: _profileNames[profile]!,
                            selected: settings.profile == profile,
                            onTap: () =>
                                shortcuts.update(settings.choose(profile)),
                          ),
                    ],
                  ),
                ),
              ),
              if (settings.profile == Profile.custom)
                Text(
                  'based on ${_profileNames[settings.base]}',
                  style: TextStyle(
                    color: ws.textFaint,
                    fontSize: WsMetrics.fsSmall,
                  ),
                ),
            ],
          ),
        ),
        if (available && settings.profile == Profile.custom)
          _Row(
            label: 'Use Caps Lock as the Liquid key',
            keys: [
              // As small as the approved mockup's switch, within the row.
              SizedBox(
                height: 20,
                child: FittedBox(
                  child: Switch(
                    value: settings.liquidKey,
                    onChanged: (on) =>
                        shortcuts.update(settings.withLiquidKey(on)),
                  ),
                ),
              ),
            ],
          ),
      ],
    );
  }
}

class _Segment extends StatelessWidget {
  const _Segment({
    required this.label,
    required this.selected,
    required this.onTap,
  });

  final String label;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return InkWell(
      onTap: onTap,
      borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
      // Ink, so the focus highlight shows above the selected segment.
      child: Ink(
        height: 24,
        padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space4),
        decoration: selected
            ? BoxDecoration(
                color: ws.canvas,
                borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                border: Border.all(color: ws.borderStrong),
              )
            : null,
        child: Center(
          widthFactor: 1,
          child: Text(
            label,
            style: TextStyle(
              fontSize: WsMetrics.fsCompact,
              color: selected ? ws.text : ws.textMuted,
              fontWeight: selected ? WsMetrics.fwMedium : WsMetrics.fwRegular,
            ),
          ),
        ),
      ),
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

/// Keys as a key cap (approved mockup).
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

/// A warning below the recording row (approved mockup).
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
    // Focus on a link shows above the notice's background.
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

/// Asks whether a change may replace the kept Custom profile (approved mockup); keeping it is
/// the default, and leaving the question keeps it too.
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
  final ({Command command, Keys keys, Command? clear}) change;
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
      bindings: {const SingleActivator(LogicalKeyboardKey.escape): onKeep},
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
                  'new Custom profile from ${_profileNames[settings.profile]}. This '
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
