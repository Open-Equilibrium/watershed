import 'dart:async';

import 'package:flutter/material.dart';

import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'workspace_state.dart';

const lucidePrefix = 'lucide:';

const lucideIcons = <String, IconData>{
  'star': IconData(0xe176, fontFamily: 'Lucide'),
  'heart': IconData(0xe0f2, fontFamily: 'Lucide'),
  'book': IconData(0xe05e, fontFamily: 'Lucide'),
  'bookmark': IconData(0xe060, fontFamily: 'Lucide'),
  'briefcase': IconData(0xe062, fontFamily: 'Lucide'),
  'calendar': IconData(0xe063, fontFamily: 'Lucide'),
  'camera': IconData(0xe064, fontFamily: 'Lucide'),
  'car': IconData(0xe1d5, fontFamily: 'Lucide'),
  'clipboard': IconData(0xe085, fontFamily: 'Lucide'),
  'cloud': IconData(0xe088, fontFamily: 'Lucide'),
  'code': IconData(0xe093, fontFamily: 'Lucide'),
  'coffee': IconData(0xe096, fontFamily: 'Lucide'),
  'compass': IconData(0xe09b, fontFamily: 'Lucide'),
  'credit-card': IconData(0xe0aa, fontFamily: 'Lucide'),
  'database': IconData(0xe0ad, fontFamily: 'Lucide'),
  'dollar-sign': IconData(0xe0b1, fontFamily: 'Lucide'),
  'file-text': IconData(0xe0cc, fontFamily: 'Lucide'),
  'flag': IconData(0xe0d1, fontFamily: 'Lucide'),
  'folder': IconData(0xe0d7, fontFamily: 'Lucide'),
  'gift': IconData(0xe0e1, fontFamily: 'Lucide'),
  'globe': IconData(0xe0e8, fontFamily: 'Lucide'),
  'graduation-cap': IconData(0xe234, fontFamily: 'Lucide'),
  'hammer': IconData(0xe0ec, fontFamily: 'Lucide'),
  'image': IconData(0xe0f6, fontFamily: 'Lucide'),
  'inbox': IconData(0xe0f7, fontFamily: 'Lucide'),
  'key': IconData(0xe0fd, fontFamily: 'Lucide'),
  'lightbulb': IconData(0xe1c2, fontFamily: 'Lucide'),
  'list-todo': IconData(0xe4c3, fontFamily: 'Lucide'),
  'lock': IconData(0xe10b, fontFamily: 'Lucide'),
  'mail': IconData(0xe10f, fontFamily: 'Lucide'),
  'map': IconData(0xe110, fontFamily: 'Lucide'),
  'message-square': IconData(0xe117, fontFamily: 'Lucide'),
  'package': IconData(0xe129, fontFamily: 'Lucide'),
  'pen-tool': IconData(0xe131, fontFamily: 'Lucide'),
  'phone': IconData(0xe133, fontFamily: 'Lucide'),
  'chart-pie': IconData(0xe06b, fontFamily: 'Lucide'),
  'plane': IconData(0xe1de, fontFamily: 'Lucide'),
  'rocket': IconData(0xe286, fontFamily: 'Lucide'),
  'settings': IconData(0xe154, fontFamily: 'Lucide'),
  'shopping-cart': IconData(0xe15c, fontFamily: 'Lucide'),
  'sparkles': IconData(0xe412, fontFamily: 'Lucide'),
  'sun': IconData(0xe178, fontFamily: 'Lucide'),
  'target': IconData(0xe180, fontFamily: 'Lucide'),
  'trophy': IconData(0xe373, fontFamily: 'Lucide'),
  'truck': IconData(0xe194, fontFamily: 'Lucide'),
  'user': IconData(0xe19f, fontFamily: 'Lucide'),
  'users': IconData(0xe1a4, fontFamily: 'Lucide'),
  'wallet': IconData(0xe204, fontFamily: 'Lucide'),
  'wrench': IconData(0xe1b1, fontFamily: 'Lucide'),
  'zap': IconData(0xe1b4, fontFamily: 'Lucide'),
};

const emojiGroups = <(String, List<(String, String)>)>[
  (
    'Objects',
    [
      ('💰', 'money'),
      ('📊', 'chart'),
      ('📈', 'growth'),
      ('📉', 'decline'),
      ('🧾', 'receipt'),
      ('💳', 'card'),
      ('🛒', 'shopping'),
      ('📦', 'package'),
      ('🎁', 'gift'),
      ('📁', 'folder'),
      ('🗂️', 'files'),
      ('📌', 'pin'),
      ('📎', 'clip'),
      ('📚', 'books'),
      ('📝', 'note'),
      ('✏️', 'pencil'),
      ('📅', 'calendar'),
      ('🗓️', 'plan'),
      ('⏰', 'alarm'),
      ('🔑', 'key'),
      ('🔒', 'lock'),
      ('💡', 'idea'),
      ('🧮', 'abacus'),
      ('🧭', 'compass'),
      ('🔖', 'bookmark'),
      ('🎒', 'backpack'),
      ('💼', 'work'),
      ('📷', 'camera'),
      ('💻', 'laptop'),
      ('📱', 'phone'),
      ('🖨️', 'printer'),
      ('🧰', 'toolbox'),
      ('🔧', 'wrench'),
      ('🧪', 'test'),
      ('🩺', 'health'),
      ('💊', 'medicine'),
    ],
  ),
  (
    'Places',
    [
      ('🏠', 'home'),
      ('🏢', 'office'),
      ('🏫', 'school'),
      ('🏥', 'hospital'),
      ('🏦', 'bank'),
      ('🏖️', 'beach'),
      ('⛰️', 'mountain'),
      ('🏙️', 'city'),
      ('🗺️', 'map'),
      ('🧳', 'travel'),
      ('✈️', 'flight'),
      ('🚆', 'train'),
      ('🚗', 'car'),
      ('🚲', 'bike'),
      ('⛵', 'boat'),
      ('🌍', 'world'),
    ],
  ),
  (
    'Nature',
    [
      ('🌱', 'plant'),
      ('🌳', 'tree'),
      ('🌸', 'flower'),
      ('🍀', 'luck'),
      ('☀️', 'sun'),
      ('🌙', 'moon'),
      ('⭐', 'star'),
      ('🔥', 'fire'),
      ('💧', 'water'),
      ('❄️', 'snow'),
      ('🌈', 'rainbow'),
      ('🐶', 'dog'),
      ('🐱', 'cat'),
      ('🐝', 'bee'),
    ],
  ),
  (
    'Food',
    [
      ('☕', 'coffee'),
      ('🍵', 'tea'),
      ('🍎', 'apple'),
      ('🥗', 'salad'),
      ('🍕', 'pizza'),
      ('🍰', 'cake'),
      ('🍷', 'wine'),
      ('🍽️', 'dinner'),
    ],
  ),
  (
    'Activities',
    [
      ('🎯', 'goal'),
      ('🏆', 'trophy'),
      ('🎉', 'party'),
      ('🎵', 'music'),
      ('🎨', 'art'),
      ('🎮', 'game'),
      ('⚽', 'football'),
      ('🏃', 'run'),
      ('🧘', 'yoga'),
      ('📖', 'reading'),
      ('🎬', 'film'),
      ('🎓', 'study'),
    ],
  ),
  (
    'Symbols',
    [
      ('✅', 'done'),
      ('❗', 'important'),
      ('❓', 'question'),
      ('💬', 'chat'),
      ('❤️', 'heart'),
      ('⚡', 'energy'),
      ('🚀', 'launch'),
      ('🧩', 'puzzle'),
      ('⚙️', 'settings'),
      ('🔔', 'bell'),
      ('📣', 'announce'),
      ('🛠️', 'tools'),
      ('🧠', 'brain'),
      ('👥', 'team'),
      ('👤', 'person'),
      ('🤝', 'deal'),
    ],
  ),
];

class PageIcon extends StatelessWidget {
  const PageIcon(this.icon, {super.key, this.size = 14, this.color});

  final String icon;
  final double size;
  final Color? color;

  @override
  Widget build(BuildContext context) {
    final tint = color ?? context.ws.textFaint;
    if (icon.startsWith(lucidePrefix)) {
      return Icon(
        lucideIcons[icon.substring(lucidePrefix.length)] ?? LucideIcons.file,
        size: size,
        color: tint,
      );
    }
    if (icon.isEmpty) return Icon(LucideIcons.file, size: size, color: tint);
    return Text(
      icon,
      style: TextStyle(fontSize: size, height: 1.15),
      textHeightBehavior: const TextHeightBehavior(
        applyHeightToFirstAscent: false,
        applyHeightToLastDescent: false,
      ),
    );
  }
}

class PageIconButton extends StatefulWidget {
  const PageIconButton({
    super.key,
    required this.state,
    required this.page,
    required this.icon,
    required this.shown,
  });

  final WorkspaceState state;
  final String page;
  final String icon;
  final bool shown;

  @override
  State<PageIconButton> createState() => _PageIconButtonState();
}

class _PageIconButtonState extends State<PageIconButton> {
  final _menu = MenuController();

  Future<void> _set(String icon) async {
    _menu.close();
    await widget.state.setPageIcon(widget.page, icon);
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return MenuAnchor(
      controller: _menu,
      alignmentOffset: const Offset(0, WsMetrics.space1),
      style: MenuStyle(
        padding: const WidgetStatePropertyAll(EdgeInsets.zero),
        backgroundColor: WidgetStatePropertyAll(ws.surface),
      ),
      menuChildren: [
        IconPicker(
          state: widget.state,
          icon: widget.icon,
          onIcon: (icon) => unawaited(_set(icon)),
        ),
      ],
      builder: (context, controller, _) => widget.icon.isEmpty
          ? AnimatedOpacity(
              opacity: widget.shown || controller.isOpen ? 1 : 0,
              duration: const Duration(milliseconds: 120),
              child: TextButton.icon(
                key: const Key('add page icon'),
                onPressed: controller.open,
                icon: Icon(LucideIcons.sparkles, size: 14, color: ws.textFaint),
                label: Text(
                  'Add icon',
                  style: TextStyle(
                    fontSize: WsMetrics.fsCompact,
                    color: ws.textFaint,
                  ),
                ),
              ),
            )
          : InkWell(
              key: const Key('page icon'),
              borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
              onTap: controller.open,
              child: Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: WsMetrics.space1,
                ),
                child: PageIcon(
                  widget.icon,
                  size: WsMetrics.fsDisplay,
                  color: ws.text,
                ),
              ),
            ),
    );
  }
}

class IconPicker extends StatefulWidget {
  const IconPicker({
    super.key,
    required this.state,
    required this.icon,
    required this.onIcon,
  });

  final WorkspaceState state;
  final String icon;
  final ValueChanged<String> onIcon;

  @override
  State<IconPicker> createState() => _IconPickerState();
}

class _IconPickerState extends State<IconPicker> {
  static const _recentKey = 'page icons';
  var _icons = false;
  var _query = '';
  List<String> _recent = const [];

  @override
  void initState() {
    super.initState();
    unawaited(_readRecent());
  }

  Future<void> _readRecent() async {
    try {
      final kept = await widget.state.workspace.remembered(key: _recentKey);
      if (!mounted || kept == null || kept.isEmpty) return;
      setState(() => _recent = kept.split(' '));
    } on Object {
      return;
    }
  }

  void _pick(String icon) {
    final recent = [icon, ..._recent.where((kept) => kept != icon)].take(9);
    unawaited(
      widget.state.workspace
          .remember(key: _recentKey, value: recent.join(' '))
          .then((_) {}, onError: (Object _) {}),
    );
    widget.onIcon(icon);
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final query = _query.trim().toLowerCase();
    final label = TextStyle(fontSize: WsMetrics.fsSmall, color: ws.textFaint);
    Widget tab(String text, bool on, VoidCallback onTap) => InkWell(
      onTap: onTap,
      child: Padding(
        padding: const EdgeInsets.symmetric(
          horizontal: WsMetrics.space2,
          vertical: WsMetrics.space1,
        ),
        child: Text(
          text,
          style: TextStyle(
            fontSize: WsMetrics.fsCompact,
            color: on ? ws.text : ws.textMuted,
            fontWeight: on ? WsMetrics.fwSemibold : WsMetrics.fwRegular,
          ),
        ),
      ),
    );
    Widget cell(String icon, {Key? key}) => InkWell(
      key: key,
      borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
      onTap: () => _pick(icon),
      child: Container(
        width: 32,
        height: 32,
        alignment: Alignment.center,
        decoration: BoxDecoration(
          color: icon == widget.icon ? ws.hover : null,
          borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        ),
        child: PageIcon(icon, size: 18, color: ws.textMuted),
      ),
    );
    final sections = <(String, List<String>)>[
      if (_icons)
        (
          'Icons',
          [
            for (final name in lucideIcons.keys)
              if (query.isEmpty || name.contains(query)) '$lucidePrefix$name',
          ],
        )
      else ...[
        if (query.isEmpty && _recent.isNotEmpty) ('Recent', _recent),
        for (final (group, emoji) in emojiGroups)
          (
            group,
            [
              for (final (icon, word) in emoji)
                if (query.isEmpty || word.contains(query)) icon,
            ],
          ),
      ],
    ].where((section) => section.$2.isNotEmpty).toList();
    return SizedBox(
      key: const Key('icon picker'),
      width: 340,
      height: 360,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(
              WsMetrics.space2,
              WsMetrics.space2,
              WsMetrics.space2,
              0,
            ),
            child: Row(
              children: [
                tab('Emoji', !_icons, () => setState(() => _icons = false)),
                tab('Icons', _icons, () => setState(() => _icons = true)),
                const Spacer(),
                if (widget.icon.isNotEmpty)
                  tab('Remove', false, () => widget.onIcon('')),
              ],
            ),
          ),
          Padding(
            padding: const EdgeInsets.all(WsMetrics.space2),
            child: TextField(
              key: const Key('icon search'),
              autofocus: true,
              onChanged: (text) => setState(() => _query = text),
              style: const TextStyle(fontSize: WsMetrics.fsCompact),
              decoration: InputDecoration(
                isDense: true,
                hintText: 'Search',
                prefixIcon: Icon(
                  LucideIcons.search,
                  size: 14,
                  color: ws.textFaint,
                ),
                prefixIconConstraints: const BoxConstraints(minWidth: 28),
                border: OutlineInputBorder(
                  borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                ),
              ),
            ),
          ),
          Expanded(
            child: sections.isEmpty
                ? Center(child: Text('Nothing found', style: label))
                : ListView(
                    primary: false,
                    padding: const EdgeInsets.fromLTRB(
                      WsMetrics.space2,
                      0,
                      WsMetrics.space2,
                      WsMetrics.space2,
                    ),
                    children: [
                      for (final (name, icons) in sections) ...[
                        Padding(
                          padding: const EdgeInsets.symmetric(
                            horizontal: WsMetrics.space1,
                            vertical: WsMetrics.space1,
                          ),
                          child: Text(name, style: label),
                        ),
                        Wrap(
                          children: [
                            for (final icon in icons)
                              cell(icon, key: ValueKey('icon $name $icon')),
                          ],
                        ),
                      ],
                    ],
                  ),
          ),
        ],
      ),
    );
  }
}
