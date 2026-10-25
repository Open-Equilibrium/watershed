import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

/// The key Liquid sees for the Liquid key: macOS turns Caps Lock into F18 while it is on
/// (ADR-0205).
const liquidKey = LogicalKeyboardKey.f18;

/// A key with the modifiers held for it; `liquid` is the Liquid key.
@immutable
class Keys {
  const Keys(
    this.key, {
    this.liquid = false,
    this.meta = false,
    this.alt = false,
    this.shift = false,
    this.control = false,
  });

  final LogicalKeyboardKey key;
  final bool liquid;
  final bool meta;
  final bool alt;
  final bool shift;
  final bool control;

  ShortcutActivator get activator => liquid
      ? _LiquidActivator(this)
      : SingleActivator(
          key,
          meta: meta,
          alt: alt,
          shift: shift,
          control: control,
        );

  /// How badges and Settings show the keys, modifiers in macOS order.
  String get label => [
    if (control) '⌃',
    if (alt) '⌥',
    if (shift) '⇧',
    if (liquid) '⇪',
    if (meta) '⌘',
    _names[key] ?? key.keyLabel.toUpperCase(),
  ].join();

  static final _names = {
    LogicalKeyboardKey.enter: '⏎',
    LogicalKeyboardKey.escape: 'esc',
    LogicalKeyboardKey.space: 'Space',
    LogicalKeyboardKey.tab: '⇥',
    LogicalKeyboardKey.backspace: '⌫',
    LogicalKeyboardKey.arrowLeft: '←',
    LogicalKeyboardKey.arrowRight: '→',
    LogicalKeyboardKey.arrowUp: '↑',
    LogicalKeyboardKey.arrowDown: '↓',
  };

  /// The keys held now with `key`, as Settings records them.
  factory Keys.pressed(LogicalKeyboardKey key, HardwareKeyboard keyboard) =>
      Keys(
        key,
        liquid: keyboard.logicalKeysPressed.contains(liquidKey),
        meta: keyboard.isMetaPressed,
        alt: keyboard.isAltPressed,
        shift: keyboard.isShiftPressed,
        control: keyboard.isControlPressed,
      );

  String encode() => [
    if (liquid) 'liquid',
    if (meta) 'meta',
    if (alt) 'alt',
    if (shift) 'shift',
    if (control) 'control',
    '0x${key.keyId.toRadixString(16)}',
  ].join('+');

  static Keys? decode(Object? text) {
    if (text is! String) return null;
    final parts = text.split('+');
    final id = parts.removeLast();
    final value = id.startsWith('0x')
        ? int.tryParse(id.substring(2), radix: 16)
        : null;
    if (value == null ||
        parts.any((part) => !_modifiers.contains(part)) ||
        parts.toSet().length != parts.length) {
      return null;
    }
    return Keys(
      LogicalKeyboardKey.findKeyByKeyId(value) ?? LogicalKeyboardKey(value),
      liquid: parts.contains('liquid'),
      meta: parts.contains('meta'),
      alt: parts.contains('alt'),
      shift: parts.contains('shift'),
      control: parts.contains('control'),
    );
  }

  static const _modifiers = {'liquid', 'meta', 'alt', 'shift', 'control'};

  @override
  bool operator ==(Object other) =>
      other is Keys &&
      other.key == key &&
      other.liquid == liquid &&
      other.meta == meta &&
      other.alt == alt &&
      other.shift == shift &&
      other.control == control;

  @override
  int get hashCode => Object.hash(key, liquid, meta, alt, shift, control);

  @override
  String toString() => label;
}

/// Accepts its key while the Liquid key is held with exactly its modifiers.
class _LiquidActivator extends ShortcutActivator {
  const _LiquidActivator(this.keys);

  final Keys keys;

  @override
  Iterable<LogicalKeyboardKey> get triggers => [keys.key];

  @override
  bool accepts(KeyEvent event, HardwareKeyboard state) =>
      event is! KeyUpEvent &&
      event.logicalKey == keys.key &&
      state.logicalKeysPressed.contains(liquidKey) &&
      state.isMetaPressed == keys.meta &&
      state.isAltPressed == keys.alt &&
      state.isShiftPressed == keys.shift &&
      state.isControlPressed == keys.control;

  @override
  String debugDescribeKeys() => keys.label;
}

/// Where a command's keys work; keys conflict only within one area.
enum Area { app, input, approval, question, trajectory }

/// Every action with keys the user can change in Settings → Shortcuts.
enum Command {
  newSession(
    'New Session',
    Area.app,
    Keys(LogicalKeyboardKey.keyN, meta: true),
    liquid: true,
  ),
  settings('Settings…', Area.app, Keys(LogicalKeyboardKey.comma, meta: true)),
  chat(
    'Chat',
    Area.app,
    Keys(LogicalKeyboardKey.digit1, meta: true),
    liquid: true,
  ),
  trajectory(
    'Trajectory',
    Area.app,
    Keys(LogicalKeyboardKey.digit2, meta: true),
    liquid: true,
  ),
  diff(
    'Diff',
    Area.app,
    Keys(LogicalKeyboardKey.digit3, meta: true),
    liquid: true,
  ),
  focusInput(
    'Focus Input',
    Area.app,
    Keys(LogicalKeyboardKey.keyL, meta: true),
    liquid: true,
  ),
  dismissNotice(
    'Dismiss a notification',
    Area.app,
    Keys(LogicalKeyboardKey.escape),
  ),
  send(
    'Send, or steer a running agent',
    Area.input,
    Keys(LogicalKeyboardKey.enter),
  ),
  queue(
    'Queue a follow-up',
    Area.input,
    Keys(LogicalKeyboardKey.enter, alt: true),
  ),
  stop('Stop the run', Area.input, Keys(LogicalKeyboardKey.escape)),
  allowOnce('Allow once', Area.approval, Keys(LogicalKeyboardKey.enter)),
  allowSession(
    'Allow for this session',
    Area.approval,
    Keys(LogicalKeyboardKey.enter, meta: true),
    liquid: true,
  ),
  reject('Reject', Area.approval, Keys(LogicalKeyboardKey.escape)),
  answer('Answer, or confirm', Area.question, Keys(LogicalKeyboardKey.enter)),
  answerEditor(
    'Answer an editor question',
    Area.question,
    Keys(LogicalKeyboardKey.enter, meta: true),
    liquid: true,
  ),
  dismissQuestion('Dismiss', Area.question, Keys(LogicalKeyboardKey.escape)),
  foldAll(
    'Fold all turns',
    Area.trajectory,
    Keys(LogicalKeyboardKey.arrowLeft, alt: true),
  ),
  openStep('Open the step', Area.trajectory, Keys(LogicalKeyboardKey.enter)),
  closeStep('Close the step', Area.trajectory, Keys(LogicalKeyboardKey.escape)),
  branch(
    'Branch from the step',
    Area.trajectory,
    Keys(LogicalKeyboardKey.keyB),
  ),
  compare('Compare branches', Area.trajectory, Keys(LogicalKeyboardKey.keyC)),
  selectBranch(
    'Select a branch to compare',
    Area.trajectory,
    Keys(LogicalKeyboardKey.space),
  );

  const Command(this.label, this.area, this.keys, {this.liquid = false});

  final String label;
  final Area area;

  /// The Standard profile's keys.
  final Keys keys;

  /// Whether the Liquid key profile adds ⇪ + the same key; platform standards such as
  /// Settings… keep ⌘ only (ADR-0205).
  final bool liquid;
}

enum Profile { liquidKey, standard, custom }

/// The chosen profile and the shortcuts changed from it (ADR-0204, ADR-0205).
@immutable
class ShortcutSettings {
  const ShortcutSettings._(
    this.profile,
    this.base,
    this.liquidKey,
    this.changes,
  );

  factory ShortcutSettings.liquidKey() =>
      const ShortcutSettings._(Profile.liquidKey, Profile.liquidKey, true, {});

  factory ShortcutSettings.standard() =>
      const ShortcutSettings._(Profile.standard, Profile.standard, false, {});

  final Profile profile;

  /// The profile Custom started from; the profile itself otherwise.
  final Profile base;

  /// Whether Caps Lock is the Liquid key.
  final bool liquidKey;

  /// Every changed command's keys, the first shown on its badge.
  final Map<Command, List<Keys>> changes;

  List<Keys> _defaults(Command command) => [
    if (command.liquid && base == Profile.liquidKey)
      Keys(
        command.keys.key,
        liquid: true,
        alt: command.keys.alt,
        shift: command.keys.shift,
        control: command.keys.control,
      ),
    command.keys,
  ];

  /// Every key combination that runs `command`.
  List<Keys> bindings(Command command) =>
      changes[command] ?? _defaults(command);

  /// Shortcut bindings that run each command's action with all of its keys.
  Map<ShortcutActivator, VoidCallback> bind(
    Map<Command, VoidCallback> actions,
  ) => {
    for (final MapEntry(key: command, value: action) in actions.entries)
      for (final keys in bindings(command)) keys.activator: action,
  };

  /// What a badge shows for `command`; empty without keys.
  String label(Command command) => bindings(command).firstOrNull?.label ?? '';

  /// A hint naming `command`'s keys and what they do; empty without keys.
  String hint(Command command, String what) {
    final keys = label(command);
    return keys.isEmpty ? '' : '$keys $what';
  }

  /// The keys the macOS menu bar can show: ⌘ without the Liquid key.
  Keys? menu(Command command) =>
      bindings(command).where((keys) => keys.meta && !keys.liquid).firstOrNull;

  bool changed(Command command) => changes.containsKey(command);

  /// The other command in `command`'s area that `keys` already runs.
  Command? conflict(Command command, Keys keys) => Command.values
      .where(
        (other) =>
            other != command &&
            other.area == command.area &&
            bindings(other).contains(keys),
      )
      .firstOrNull;

  /// Gives `command` new first keys, or none, keeping the ⌘ keys beside the Liquid key;
  /// `clear` gives up the same keys.
  ShortcutSettings change(Command command, Keys? keys, {Command? clear}) {
    final next = {...changes};
    void put(Command command, List<Keys> list) {
      if (listEquals(list, _defaults(command))) {
        next.remove(command);
      } else {
        next[command] = list;
      }
    }

    final rest = bindings(command).skip(1).where((other) => other != keys);
    put(command, [?keys, ...rest]);
    if (clear != null && keys != null) {
      put(clear, [...bindings(clear).where((other) => other != keys)]);
    }
    return ShortcutSettings._(Profile.custom, base, liquidKey, next);
  }

  ShortcutSettings reset(Command command) => ShortcutSettings._(
    profile,
    base,
    liquidKey,
    {...changes}..remove(command),
  );

  ShortcutSettings resetAll() => choose(base);

  ShortcutSettings withLiquidKey(bool on) =>
      ShortcutSettings._(Profile.custom, base, on, changes);

  /// Liquid key and Standard start over; Custom keeps what was changed.
  ShortcutSettings choose(Profile profile) => switch (profile) {
    Profile.liquidKey => ShortcutSettings.liquidKey(),
    Profile.standard => ShortcutSettings.standard(),
    Profile.custom => ShortcutSettings._(
      Profile.custom,
      base,
      liquidKey,
      changes,
    ),
  };

  Map<String, Object?> toJson() => {
    'version': 1,
    'profile': profile.name,
    'base': base.name,
    'liquidKey': liquidKey,
    'changes': {
      for (final entry in changes.entries)
        entry.key.name: [for (final keys in entry.value) keys.encode()],
    },
  };

  /// The settings in `json`; unknown commands and malformed keys fall back to their
  /// defaults, an unknown profile to none.
  static ShortcutSettings? fromJson(Map<String, Object?> json) {
    final profile = Profile.values.asNameMap()[json['profile']];
    final base = Profile.values.asNameMap()[json['base']];
    final liquidKey = json['liquidKey'];
    if (profile == null ||
        base == null ||
        base == Profile.custom ||
        liquidKey is! bool) {
      return null;
    }
    final commands = Command.values.asNameMap();
    final changes = <Command, List<Keys>>{};
    if (json['changes'] case final Map<String, Object?> stored) {
      for (final MapEntry(:key, :value) in stored.entries) {
        final command = commands[key];
        if (command == null || value is! List) continue;
        final keys = value.map(Keys.decode).toList();
        if (keys.contains(null)) continue;
        changes[command] = keys.nonNulls.toList();
      }
    }
    return ShortcutSettings._(profile, base, liquidKey, changes);
  }

  @override
  bool operator ==(Object other) =>
      other is ShortcutSettings &&
      other.profile == profile &&
      other.base == base &&
      other.liquidKey == liquidKey &&
      mapEquals(other._encoded, _encoded);

  Map<Command, String> get _encoded => changes.map(
    (command, keys) =>
        MapEntry(command, keys.map((keys) => keys.encode()).join(' ')),
  );

  @override
  int get hashCode => Object.hash(profile, base, liquidKey, changes.length);
}

/// Gives the widgets below the shortcut settings in use.
class Keymap extends InheritedNotifier<ValueListenable<ShortcutSettings>> {
  const Keymap({super.key, required super.notifier, required super.child});

  /// The settings in use; Standard without a [Keymap] above `context`.
  static ShortcutSettings of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<Keymap>()?.notifier?.value ??
      _standard;

  static final _standard = ShortcutSettings.standard();
}
