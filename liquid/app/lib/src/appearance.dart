import 'dart:io';

import 'package:flutter/material.dart';

import 'controls.dart';
import 'settings_file.dart';
import 'theme/theme.dart';
import 'theme/tokens.g.dart';

/// Which theme Liquid uses.
enum ThemeChoice {
  system(ThemeMode.system),
  light(ThemeMode.light),
  dark(ThemeMode.dark);

  const ThemeChoice(this.mode);

  final ThemeMode mode;
}

/// How large Liquid draws its text and rows (approved look mockup): Default is the base size,
/// Compact and Large scale text and rows together.
enum TextSize {
  compact(13 / 15),
  standard(1),
  large(17 / 15);

  const TextSize(this.scale);

  final double scale;
}

/// Settings → Appearance; kept across restarts.
@immutable
class Appearance {
  const Appearance({
    this.theme = ThemeChoice.system,
    this.textSize = TextSize.standard,
  });

  /// Reads what it knows from `json` and keeps the defaults for the rest.
  factory Appearance.fromJson(Map<String, Object?> json) => Appearance(
    theme: ThemeChoice.values.asNameMap()[json['theme']] ?? ThemeChoice.system,
    textSize:
        TextSize.values.asNameMap()[json['textSize']] ?? TextSize.standard,
  );

  final ThemeChoice theme;
  final TextSize textSize;

  Appearance copyWith({ThemeChoice? theme, TextSize? textSize}) => Appearance(
    theme: theme ?? this.theme,
    textSize: textSize ?? this.textSize,
  );

  Map<String, Object?> toJson() => {
    'theme': theme.name,
    'textSize': textSize.name,
  };
}

/// The appearance in use; each change applies at once and is saved.
class AppearanceController extends ValueNotifier<Appearance> {
  AppearanceController([super.value = const Appearance(), this._file]);

  static Future<AppearanceController> load(SettingsFile file) async {
    final json = await file.load();
    return AppearanceController(
      json == null ? const Appearance() : Appearance.fromJson(json),
      file,
    );
  }

  final SettingsFile? _file;

  /// The latest save; each waits for the one before, so the last change is the one kept.
  Future<void> saved = Future.value();

  /// Uses `appearance` at once; fails if it could not be saved.
  Future<void> change(Appearance appearance) {
    value = appearance;
    final file = _file;
    if (file == null) return Future.value();
    final saving = saved.then((_) => file.save(appearance.toJson()));
    saved = saving.then((_) {}, onError: (Object _) {});
    return saving;
  }
}

/// Draws `child` at `scale`, laying it out in correspondingly less room, so text, rows and
/// panels grow or shrink together.
class UiScale extends StatelessWidget {
  const UiScale({super.key, required this.scale, required this.child});

  final double scale;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    if (scale == 1) return child;
    return LayoutBuilder(
      builder: (context, constraints) {
        final size = Size(
          constraints.maxWidth / scale,
          constraints.maxHeight / scale,
        );
        return FittedBox(
          fit: BoxFit.fill,
          alignment: Alignment.topLeft,
          child: SizedBox.fromSize(
            size: size,
            child: MediaQuery(
              data: MediaQuery.of(context).copyWith(size: size),
              child: child,
            ),
          ),
        );
      },
    );
  }
}

/// Settings → Appearance (approved look mockup): the theme and the text size, applied at once.
class AppearancePane extends StatefulWidget {
  const AppearancePane({super.key, required this.appearance});

  final AppearanceController appearance;

  @override
  State<AppearancePane> createState() => _AppearancePaneState();
}

class _AppearancePaneState extends State<AppearancePane> {
  /// Why the latest change could not be saved; it still applies until Liquid quits.
  String? _failure;

  void _change(Appearance next) {
    setState(() => _failure = null);
    widget.appearance.change(next).catchError((Object error) {
      if (mounted) {
        setState(
          () => _failure = switch (error) {
            FileSystemException(:final message) =>
              'Could not save the appearance: $message',
            _ => 'Could not save the appearance: $error',
          },
        );
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    final appearance = widget.appearance;
    final ws = context.ws;
    final label = TextStyle(color: ws.textMuted, fontSize: WsMetrics.fsCompact);
    Widget row(String title, Widget control) => Padding(
      padding: const EdgeInsets.only(bottom: WsMetrics.space4),
      child: Row(
        children: [
          SizedBox(width: 120, child: Text(title, style: label)),
          control,
        ],
      ),
    );
    return ValueListenableBuilder(
      valueListenable: appearance,
      builder: (context, chosen, _) => SingleChildScrollView(
        padding: const EdgeInsets.fromLTRB(
          WsMetrics.space6,
          WsMetrics.space5,
          WsMetrics.space6,
          WsMetrics.space5,
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              'Appearance',
              style: Theme.of(context).textTheme.titleMedium
                  ?.copyWith(fontWeight: WsMetrics.fwSemibold),
            ),
            const SizedBox(height: WsMetrics.space5),
            row(
              'Theme',
              Segments(
                options: const [
                  (ThemeChoice.system, 'System'),
                  (ThemeChoice.light, 'Light'),
                  (ThemeChoice.dark, 'Dark'),
                ],
                selected: chosen.theme,
                onSelect: (theme) => _change(chosen.copyWith(theme: theme)),
              ),
            ),
            row(
              'Text size',
              Segments(
                options: const [
                  (TextSize.compact, 'Compact'),
                  (TextSize.standard, 'Default'),
                  (TextSize.large, 'Large'),
                ],
                selected: chosen.textSize,
                onSelect: (size) => _change(chosen.copyWith(textSize: size)),
              ),
            ),
            if (_failure case final failure?)
              Text(
                failure,
                style: TextStyle(color: ws.danger, fontSize: WsMetrics.fsSmall),
              ),
          ],
        ),
      ),
    );
  }
}
