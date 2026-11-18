import 'package:flutter/material.dart';

import 'agent/session_card_rows.dart';
import 'agent/session_card_settings.dart';
import 'controls.dart';
import 'settings_file.dart';
import 'theme/theme.dart';
import 'theme/tokens.g.dart';

enum ThemeChoice {
  system(ThemeMode.system),
  light(ThemeMode.light),
  dark(ThemeMode.dark);

  const ThemeChoice(this.mode);

  final ThemeMode mode;
}

enum TextSize {
  compact(13 / 14),
  standard(1),
  large(16 / 14);

  const TextSize(this.scale);

  final double scale;
}

@immutable
class Appearance {
  const Appearance({
    this.theme = ThemeChoice.system,
    this.textSize = TextSize.standard,
  });

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
        final media = MediaQuery.of(context);
        return FittedBox(
          fit: BoxFit.fill,
          alignment: Alignment.topLeft,
          child: SizedBox.fromSize(
            size: size,
            child: MediaQuery(
              data: media.copyWith(
                size: size,
                padding: media.padding / scale,
                viewPadding: media.viewPadding / scale,
                viewInsets: media.viewInsets / scale,
                systemGestureInsets: media.systemGestureInsets / scale,
              ),
              child: child,
            ),
          ),
        );
      },
    );
  }
}

class AppearancePane extends StatelessWidget {
  const AppearancePane({
    super.key,
    required this.appearance,
    this.cardSettings,
  });

  final AppearanceController appearance;

  final SessionCardSettingsController? cardSettings;

  void _change(Appearance Function(Appearance appearance) change) =>
      appearance.change(change(appearance.value));

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final label = TextStyle(color: ws.textMuted, fontSize: WsMetrics.fsCompact);
    Widget row(String title, Widget control, {String? note}) => Padding(
      padding: const EdgeInsets.only(bottom: WsMetrics.space4),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              SizedBox(width: 120, child: Text(title, style: label)),
              control,
            ],
          ),
          if (note case final text?)
            Padding(
              padding: const EdgeInsets.only(left: 120, top: WsMetrics.space2),
              child: Text(
                text,
                style: TextStyle(
                  color: ws.textFaint,
                  fontSize: WsMetrics.fsSmall,
                ),
              ),
            ),
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
            if (cardSettings case final cards?)
              row(
                'Thread details',
                _ThreadDetailsMenu(settings: cards),
                note: 'Choose what the thread details card shows. You can also right-click the card.',
              ),
          ],
        ),
      ),
    );
  }
}

class _ThreadDetailsMenu extends StatefulWidget {
  const _ThreadDetailsMenu({required this.settings});

  final SessionCardSettingsController settings;

  @override
  State<_ThreadDetailsMenu> createState() => _ThreadDetailsMenuState();
}

class _ThreadDetailsMenuState extends State<_ThreadDetailsMenu> {
  final _menu = MenuController();
  final _button = FocusNode(debugLabel: 'thread details');
  final _first = FocusNode(debugLabel: 'thread details menu');

  @override
  void dispose() {
    _button.dispose();
    _first.dispose();
    super.dispose();
  }

  void _open() {
    _menu.open();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (_menu.isOpen) _first.requestFocus();
    });
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.settings,
    builder: (context, _) {
      final shown = CardItem.values.where(widget.settings.value.shows).length;
      final count = '$shown of ${CardItem.values.length} on';
      return MenuAnchor(
        controller: _menu,
        style: MenuStyle(
          backgroundColor: WidgetStatePropertyAll(
            Theme.of(context).colorScheme.surface,
          ),
        ),
        menuChildren: threadDetailsMenuItems(
          context,
          widget.settings,
          first: _first,
        ),
        builder: (context, controller, _) => SecondaryButton(
          label: count,
          semanticsLabel: 'Thread details: $count',
          focusNode: _button,
          onPressed: _open,
        ),
      );
    },
  );
}
