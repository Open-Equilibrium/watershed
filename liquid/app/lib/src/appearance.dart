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

/// How large Liquid draws its text and rows: Default is the base size,
/// Compact and Large scale text and rows together.
enum TextSize {
  compact(13 / 14),
  standard(1),
  large(16 / 14);

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
class AppearanceController extends SavedSettings<Appearance> {
  AppearanceController([
    Appearance value = const Appearance(),
    SettingsFile? file,
  ]) : super(
         value,
         file == null ? null : (appearance) => file.save(appearance.toJson()),
         'the appearance',
       );

  static Future<AppearanceController> load(SettingsFile file) async {
    final json = await file.load();
    return AppearanceController(
      json == null ? const Appearance() : Appearance.fromJson(json),
      file,
    );
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

/// Settings → Appearance: the theme and the text size, applied at once.
class AppearancePane extends StatelessWidget {
  const AppearancePane({super.key, required this.appearance});

  final AppearanceController appearance;

  /// Changes the appearance in use now, so two choices before the next frame both apply.
  void _change(Appearance Function(Appearance appearance) change) =>
      appearance.change(change(appearance.value));

  @override
  Widget build(BuildContext context) {
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
                onSelect: (theme) => _change((it) => it.copyWith(theme: theme)),
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
                onSelect: (size) =>
                    _change((it) => it.copyWith(textSize: size)),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
