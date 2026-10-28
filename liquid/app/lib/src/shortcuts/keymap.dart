import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

/// The key Liquid sees for the Liquid key: macOS turns Caps Lock into F18 while it is
/// on.
const liquidKey = LogicalKeyboardKey.f18;

/// Esc pressed afresh and alone, which closes what is open.
final ShortcutActivator escapeKey = const Keys(LogicalKeyboardKey.escape)
    .activator;

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

  /// Accepts exactly these keys when pressed: a binding without the Liquid key ignores its
  /// key while the Liquid key is held, so ⇪⏎ never also runs ⏎. A key held from before, such
  /// as one that just decided an approval, runs nothing.
  ShortcutActivator get activator => _KeysActivator(this);

  /// Like [activator], and also while the key is held, as for moving through a list.
  ShortcutActivator get repeating => _KeysActivator(this, repeats: true);

  /// These keys as the macOS menu bar shows them; none with the Liquid key.
  SingleActivator? get menuShortcut => liquid
      ? null
      : SingleActivator(
          key,
          meta: meta,
          alt: alt,
          shift: shift,
          control: control,
        );

  /// Whether these keys type or edit in a text field: a character, alone or with ⇧ or ⌥, a
  /// ⌃ character, which moves the cursor, or a cursor or delete key with any modifier.
  bool get types =>
      !liquid &&
      (_editing.contains(key) || (!meta && key.keyLabel.runes.length == 1));

  static final _editing = {
    LogicalKeyboardKey.arrowLeft,
    LogicalKeyboardKey.arrowRight,
    LogicalKeyboardKey.arrowUp,
    LogicalKeyboardKey.arrowDown,
    LogicalKeyboardKey.backspace,
    LogicalKeyboardKey.delete,
    LogicalKeyboardKey.tab,
    LogicalKeyboardKey.home,
    LogicalKeyboardKey.end,
    LogicalKeyboardKey.pageUp,
    LogicalKeyboardKey.pageDown,
  };

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

/// Accepts its key with exactly its modifiers, the Liquid key among them.
class _KeysActivator extends ShortcutActivator {
  const _KeysActivator(this.keys, {this.repeats = false});

  final Keys keys;
  final bool repeats;

  @override
  Iterable<LogicalKeyboardKey> get triggers => [keys.key];

  @override
  bool accepts(KeyEvent event, HardwareKeyboard state) =>
      (event is KeyDownEvent || (repeats && event is KeyRepeatEvent)) &&
      event.logicalKey == keys.key &&
      state.logicalKeysPressed.contains(liquidKey) == keys.liquid &&
      state.isMetaPressed == keys.meta &&
      state.isAltPressed == keys.alt &&
      state.isShiftPressed == keys.shift &&
      state.isControlPressed == keys.control;

  @override
  String debugDescribeKeys() => keys.label;
}

/// Where a command's keys work; keys conflict only within one area.
enum Area { app, input, approval, question, trajectory, diff }

/// Keys that never change, as Settings → Shortcuts lists them per area; no command may take
/// them there.
enum Fixed {
  newLine('New line', Area.input, '⇧⏎', [
    Keys(LogicalKeyboardKey.enter, shift: true),
  ]),
  scrollCall('Scroll the call', Area.approval, '↑↓ ⇞⇟', [
    Keys(LogicalKeyboardKey.arrowUp),
    Keys(LogicalKeyboardKey.arrowDown),
    Keys(LogicalKeyboardKey.pageUp),
    Keys(LogicalKeyboardKey.pageDown),
  ]),
  moveThroughOptions('Move through options', Area.question, '↑↓', [
    Keys(LogicalKeyboardKey.arrowUp),
    Keys(LogicalKeyboardKey.arrowDown),
  ]),
  previousOrNextStep('Previous or next step', Area.trajectory, '↑↓', [
    Keys(LogicalKeyboardKey.arrowUp),
    Keys(LogicalKeyboardKey.arrowDown),
  ]),
  foldTurn('Fold or unfold the turn', Area.trajectory, '←→', [
    Keys(LogicalKeyboardKey.arrowLeft),
    Keys(LogicalKeyboardKey.arrowRight),
  ]),
  moveInTree('Move in the branch tree', Area.trajectory, '⌥↑↓', [
    Keys(LogicalKeyboardKey.arrowUp, alt: true),
    Keys(LogicalKeyboardKey.arrowDown, alt: true),
  ]),
  previousOrNextFile('Previous or next file', Area.diff, '↑↓', [
    Keys(LogicalKeyboardKey.arrowUp),
    Keys(LogicalKeyboardKey.arrowDown),
  ]),
  foldFolder('Fold or unfold the folder', Area.diff, '←→', [
    Keys(LogicalKeyboardKey.arrowLeft),
    Keys(LogicalKeyboardKey.arrowRight),
  ]),
  previousOrNextHunk('Previous or next change', Area.diff, '⌥↑↓', [
    Keys(LogicalKeyboardKey.arrowUp, alt: true),
    Keys(LogicalKeyboardKey.arrowDown, alt: true),
  ]),
  unifiedOrSplit('Unified or split', Area.diff, 'U', [
    Keys(LogicalKeyboardKey.keyU),
  ], repeats: false),
  hideWhitespace('Hide or show whitespace changes', Area.diff, 'W', [
    Keys(LogicalKeyboardKey.keyW),
  ], repeats: false);

  const Fixed(
    this.label,
    this.area,
    this.display,
    this.keys, {
    this.repeats = true,
  });

  final String label;
  final Area area;

  /// The keys as Settings show them.
  final String display;
  final List<Keys> keys;

  /// Whether a held key acts again, as for moving through a list; a switch acts once.
  final bool repeats;

  /// Binds each key to the action at its position, such as ↑ to back and ↓ to on.
  Map<ShortcutActivator, VoidCallback> bind(List<VoidCallback> actions) {
    assert(actions.length == keys.length, '$name binds ${keys.length} keys');
    return {
      for (final (index, keys) in this.keys.indexed)
        repeats ? keys.repeating : keys.activator: actions[index],
    };
  }
}

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
  /// Settings… keep ⌘ only.
  final bool liquid;
}

enum Profile {
  liquidKey('Liquid key'),
  standard('Standard'),
  custom('Custom');

  const Profile(this.label);

  /// The profile's name, as Liquid shows it.
  final String label;
}

/// What a command's keys may not be: keys a fixed row or macOS uses, or keys that type where the
/// command works; null when the keys are free.
String? reserved(Command command, Keys keys) {
  // The menu bar ignores the Liquid key, so ⇪⌘Q still quits.
  final menu = Keys(
    keys.key,
    meta: keys.meta,
    alt: keys.alt,
    shift: keys.shift,
    control: keys.control,
  );
  for (final (area, reserved, use) in _reserved) {
    if (area == null
        ? reserved == menu
        : area == command.area && reserved == keys) {
      return use;
    }
  }
  final typing = {Area.app, Area.input, Area.question}.contains(command.area);
  return typing && keys.types ? 'typing' : null;
}

/// macOS standards everywhere, and the fixed keys per area.
final _reserved = <(Area?, Keys, String)>[
  (null, const Keys(LogicalKeyboardKey.keyQ, meta: true), 'Quit'),
  (null, const Keys(LogicalKeyboardKey.keyW, meta: true), 'Close'),
  (null, const Keys(LogicalKeyboardKey.keyH, meta: true), 'Hide'),
  (
    null,
    const Keys(LogicalKeyboardKey.keyH, meta: true, alt: true),
    'Hide Others',
  ),
  (null, const Keys(LogicalKeyboardKey.keyM, meta: true), 'Minimize'),
  (
    null,
    const Keys(LogicalKeyboardKey.keyF, meta: true, control: true),
    'Full Screen',
  ),
  (null, const Keys(LogicalKeyboardKey.keyC, meta: true), 'Copy'),
  (null, const Keys(LogicalKeyboardKey.keyV, meta: true), 'Paste'),
  (null, const Keys(LogicalKeyboardKey.keyX, meta: true), 'Cut'),
  (null, const Keys(LogicalKeyboardKey.keyA, meta: true), 'Select All'),
  (null, const Keys(LogicalKeyboardKey.keyZ, meta: true), 'Undo'),
  (null, const Keys(LogicalKeyboardKey.keyZ, meta: true, shift: true), 'Redo'),
  for (final fixed in Fixed.values)
    for (final keys in fixed.keys) (fixed.area, keys, fixed.label),
];

/// The Custom profile: the profile it started from, whether Caps Lock is the Liquid key and
/// every changed command's keys, the first shown on its badge.
@immutable
class CustomProfile {
  const CustomProfile(this.base, this.liquidKey, this.changes);

  final Profile base;
  final bool liquidKey;
  final Map<Command, List<Keys>> changes;

  Map<Command, String> get _encoded => changes.map(
    (command, keys) =>
        MapEntry(command, keys.map((keys) => keys.encode()).join(' ')),
  );

  @override
  bool operator ==(Object other) =>
      other is CustomProfile &&
      other.base == base &&
      other.liquidKey == liquidKey &&
      mapEquals(other._encoded, _encoded);

  @override
  int get hashCode => Object.hash(base, liquidKey, changes.length);
}

/// The active profile, and the Custom profile kept while another one is active, so choosing
/// Custom again brings it back.
@immutable
class ShortcutSettings {
  const ShortcutSettings._(this.profile, this.custom);

  factory ShortcutSettings.liquidKey() =>
      const ShortcutSettings._(Profile.liquidKey, null);

  factory ShortcutSettings.standard() =>
      const ShortcutSettings._(Profile.standard, null);

  final Profile profile;

  /// The Custom profile, if the user made one.
  final CustomProfile? custom;

  CustomProfile? get _active => profile == Profile.custom ? custom : null;

  /// The profile Custom started from; the profile itself otherwise.
  Profile get base => _active?.base ?? profile;

  /// Whether Caps Lock is the Liquid key.
  bool get liquidKey => _active?.liquidKey ?? profile == Profile.liquidKey;

  /// Every changed command's keys in the active profile.
  Map<Command, List<Keys>> get changes => _active?.changes ?? const {};

  List<Keys> _defaults(Command command) => [
    if (command.liquid && liquidKey)
      Keys(
        command.keys.key,
        liquid: true,
        alt: command.keys.alt,
        shift: command.keys.shift,
        control: command.keys.control,
      ),
    command.keys,
  ];

  /// Every key combination that runs `command`; Liquid key combinations only while it is on.
  List<Keys> bindings(Command command) => [
    for (final keys in changes[command] ?? _defaults(command))
      if (liquidKey || !keys.liquid) keys,
  ];

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

  /// What `command` shows when unchanged.
  String defaultLabel(Command command) => _defaults(command).first.label;

  /// The other command in `command`'s area that `keys` already runs.
  Command? conflict(Command command, Keys keys) => Command.values
      .where(
        (other) =>
            other != command &&
            other.area == command.area &&
            bindings(other).contains(keys),
      )
      .firstOrNull;

  /// The kept Custom profile a change here would replace, when it holds anything its base
  /// does not.
  CustomProfile? get replaced => switch (custom) {
    final kept?
        when profile != Profile.custom &&
            (kept.changes.isNotEmpty ||
                kept.liquidKey != (kept.base == Profile.liquidKey)) =>
      kept,
    _ => null,
  };

  /// The Custom profile to change: the active one, or a new one based on the active profile.
  CustomProfile get _editable =>
      _active ?? CustomProfile(profile, liquidKey, const {});

  /// Gives `command` new first keys, or none, keeping the ⌘ keys beside the Liquid key;
  /// `clear` gives up the same keys. Changing a profile other than Custom starts a new Custom
  /// profile from it.
  ShortcutSettings change(Command command, Keys? keys, {Command? clear}) {
    final custom = _editable;
    final from = ShortcutSettings._(Profile.custom, custom);
    final next = {...custom.changes};
    void put(Command command, List<Keys> list) {
      if (listEquals(list, from._defaults(command))) {
        next.remove(command);
      } else {
        next[command] = list;
      }
    }

    final rest = from.bindings(command).skip(1).where((other) => other != keys);
    put(command, [?keys, ...rest]);
    if (clear != null && keys != null) {
      put(clear, [...from.bindings(clear).where((other) => other != keys)]);
    }
    return ShortcutSettings._(
      Profile.custom,
      CustomProfile(custom.base, custom.liquidKey, next),
    );
  }

  ShortcutSettings reset(Command command) => switch (_active) {
    final custom? => ShortcutSettings._(
      Profile.custom,
      CustomProfile(
        custom.base,
        custom.liquidKey,
        {...custom.changes}..remove(command),
      ),
    ),
    null => this,
  };

  /// Drops every change; a Custom profile that then matches its base becomes that profile, so
  /// resetting never turns the Liquid key on or off.
  ShortcutSettings resetAll() => switch (_active) {
    final custom? when custom.liquidKey == (custom.base == Profile.liquidKey) =>
      ShortcutSettings._(custom.base, null),
    final custom? => ShortcutSettings._(
      Profile.custom,
      CustomProfile(custom.base, custom.liquidKey, const {}),
    ),
    null => this,
  };

  ShortcutSettings withLiquidKey(bool on) {
    final custom = _editable;
    return ShortcutSettings._(
      Profile.custom,
      CustomProfile(custom.base, on, custom.changes),
    );
  }

  /// These settings without the Liquid key: Standard for the Liquid key profile, Custom with
  /// its switch off otherwise.
  ShortcutSettings withoutLiquidKey() => switch (profile) {
    _ when !liquidKey => this,
    Profile.liquidKey => ShortcutSettings._(Profile.standard, custom),
    _ => withLiquidKey(false),
  };

  /// Activates `profile`; Custom comes back as it was left, or starts from the active profile.
  ShortcutSettings choose(Profile profile) => switch (profile) {
    Profile.custom => ShortcutSettings._(Profile.custom, custom ?? _editable),
    _ => ShortcutSettings._(profile, custom),
  };

  Map<String, Object?> toJson() => {
    'version': 1,
    'profile': profile.name,
    if (custom case final custom?)
      'custom': {
        'base': custom.base.name,
        'liquidKey': custom.liquidKey,
        'changes': {
          for (final entry in custom.changes.entries)
            entry.key.name: [for (final keys in entry.value) keys.encode()],
        },
      },
  };

  /// The settings in `json`; unknown commands and malformed keys fall back to their defaults.
  /// Another version, an unknown profile or Custom without its profile yields none.
  static ShortcutSettings? fromJson(Map<String, Object?> json) {
    final profile = Profile.values.asNameMap()[json['profile']];
    if (json['version'] != 1 || profile == null) return null;
    final custom = _custom(json['custom']);
    if (profile == Profile.custom && custom == null) return null;
    return ShortcutSettings._(profile, custom);
  }

  static CustomProfile? _custom(Object? json) {
    if (json is! Map<String, Object?>) return null;
    final base = Profile.values.asNameMap()[json['base']];
    final liquidKey = json['liquidKey'];
    if (base == null || base == Profile.custom || liquidKey is! bool) {
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
        // Keys listed twice would run the command twice.
        changes[command] = keys.nonNulls.toSet().toList();
      }
    }
    return CustomProfile(base, liquidKey, changes);
  }

  @override
  bool operator ==(Object other) =>
      other is ShortcutSettings &&
      other.profile == profile &&
      other.custom == custom;

  @override
  int get hashCode => Object.hash(profile, custom);
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
