import 'package:flutter/material.dart';

import '../rust/api/workspace.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';

/// A slash command: a Text Block style, or a Table without one.
@immutable
class SlashCommand {
  const SlashCommand(
    this.label,
    this.icon, {
    this.style,
    this.shortcut,
    this.description,
  });

  final String label;
  final IconData icon;
  final BridgeTextStyle? style;

  /// What it holds, under its label.
  final String? description;

  /// The Markdown shortcut that formats the same way.
  final String? shortcut;
}

/// The slash commands in order: the Text Block styles, then Table.
const slashCommands = [
  SlashCommand('Text', LucideIcons.text, style: BridgeTextStyle.paragraph),
  SlashCommand(
    'Heading 1',
    LucideIcons.heading1,
    style: BridgeTextStyle.heading1,
    shortcut: '#',
  ),
  SlashCommand(
    'Heading 2',
    LucideIcons.heading2,
    style: BridgeTextStyle.heading2,
    shortcut: '##',
  ),
  SlashCommand(
    'Heading 3',
    LucideIcons.heading3,
    style: BridgeTextStyle.heading3,
    shortcut: '###',
  ),
  SlashCommand(
    'Bulleted list',
    LucideIcons.list,
    style: BridgeTextStyle.bullet,
    shortcut: '-',
  ),
  SlashCommand(
    'Numbered list',
    LucideIcons.listOrdered,
    style: BridgeTextStyle.numbered,
    shortcut: '1.',
  ),
  SlashCommand(
    'To-do list',
    LucideIcons.squareCheck,
    style: BridgeTextStyle.todo,
    shortcut: '[]',
  ),
  SlashCommand(
    'Quote',
    LucideIcons.textQuote,
    style: BridgeTextStyle.quote,
    shortcut: '"',
  ),
  SlashCommand('Table', LucideIcons.table, description: 'Fields and rows'),
];

/// The choices whose label holds `query`, ignoring case.
List<SlashCommand> slashCommandsFor(String query) => [
  for (final choice in slashCommands)
    if (choice.label.toLowerCase().contains(query.toLowerCase())) choice,
];

/// The slash command menu that `/` opens in an empty Text Block, as in Notion and T3 Code
/// (approved Table View
/// mockups, section 2): the Text Block styles, then Table, filtered by what follows `/`.
class SlashCommands extends StatefulWidget {
  const SlashCommands({
    super.key,
    required this.choices,
    required this.chosen,
    required this.onChoose,
    required this.onHover,
  });

  final List<SlashCommand> choices;

  /// The choice ⏎ takes.
  final int chosen;
  final ValueChanged<SlashCommand> onChoose;
  final ValueChanged<int> onHover;

  static const _labelHeight = 26.0;
  static const _footerHeight = 26.0;

  /// A command's height, taller with a description below its label.
  static double _itemHeight(SlashCommand choice) =>
      choice.description == null ? 36 : 40;

  /// The height the menu takes with `choices`.
  static double heightFor(List<SlashCommand> choices) =>
      2 * WsMetrics.space2 +
      {for (final choice in choices) choice.style == null}.length *
          _labelHeight +
      choices.fold(0.0, (height, choice) => height + _itemHeight(choice)) +
      WsMetrics.space3 +
      _footerHeight;

  @override
  State<SlashCommands> createState() => _SlashCommandsState();
}

class _SlashCommandsState extends State<SlashCommands> {
  final _list = ScrollController();

  @override
  void initState() {
    super.initState();
    _reveal();
  }

  @override
  void didUpdateWidget(SlashCommands old) {
    super.didUpdateWidget(old);
    if (widget.chosen != old.chosen || widget.choices != old.choices) _reveal();
  }

  @override
  void dispose() {
    _list.dispose();
    super.dispose();
  }

  /// Scrolls the commands to keep the chosen one in view, with its group's label when it
  /// is the group's first.
  void _reveal() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted || !_list.hasClients) return;
      final choices = widget.choices;
      final index = widget.chosen;
      if (index >= choices.length) return;
      final choice = choices[index];
      final table = choice.style == null;
      final labels = table && choices.first.style != null ? 2 : 1;
      final item =
          labels * SlashCommands._labelHeight +
          choices
              .take(index)
              .fold(
                0.0,
                (top, above) => top + SlashCommands._itemHeight(above),
              );
      final first = index == 0 || (choices[index - 1].style == null) != table;
      final position = _list.position;
      final top = first ? item - SlashCommands._labelHeight : item;
      final bottom =
          item + SlashCommands._itemHeight(choice) - position.viewportDimension;
      if (top < position.pixels) position.jumpTo(top);
      if (bottom > position.pixels) position.jumpTo(bottom);
    });
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final SlashCommands(:choices, :chosen, :onChoose, :onHover) = widget;
    Widget label(String text) => Container(
      height: SlashCommands._labelHeight,
      alignment: Alignment.bottomLeft,
      padding: const EdgeInsets.fromLTRB(
        WsMetrics.space3,
        WsMetrics.space2,
        WsMetrics.space3,
        2,
      ),
      child: Text(
        text,
        style: TextStyle(fontSize: WsMetrics.fsSmall, color: ws.textFaint),
      ),
    );
    final texts = choices.where((choice) => choice.style != null);
    final tables = choices.where((choice) => choice.style == null);
    Widget item(SlashCommand choice) {
      final index = choices.indexOf(choice);
      return MouseRegion(
        key: ValueKey('choice ${choice.label}'),
        onEnter: (_) => onHover(index),
        child: GestureDetector(
          onTap: () => onChoose(choice),
          child: Container(
            height: SlashCommands._itemHeight(choice),
            padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
            decoration: BoxDecoration(
              color: index == chosen ? ws.hover : null,
              borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
            ),
            child: Row(
              children: [
                Container(
                  width: 28,
                  height: 28,
                  decoration: BoxDecoration(
                    border: Border.all(color: ws.border),
                    borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                  ),
                  child: Icon(choice.icon, size: 14, color: ws.textMuted),
                ),
                const SizedBox(width: 10),
                Expanded(
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(choice.label, style: TextStyle(color: ws.text)),
                      if (choice.description case final description?)
                        Text(
                          description,
                          style: TextStyle(
                            fontSize: WsMetrics.fsSmall,
                            color: ws.textFaint,
                          ),
                        ),
                    ],
                  ),
                ),
                if (choice.shortcut case final shortcut?)
                  Text(
                    shortcut,
                    style: TextStyle(
                      fontSize: WsMetrics.fsSmall,
                      color: ws.textFaint,
                    ),
                  ),
              ],
            ),
          ),
        ),
      );
    }

    return Container(
      key: const Key('slash commands'),
      width: 300,
      padding: const EdgeInsets.all(WsMetrics.space2),
      decoration: BoxDecoration(
        color: ws.surface,
        borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
        boxShadow: ws.shadow,
      ),
      child: DefaultTextStyle.merge(
        style: const TextStyle(fontSize: WsMetrics.fsUi),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            // The commands scroll in the height the menu has.
            Flexible(
              child: SingleChildScrollView(
                controller: _list,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    if (texts.isNotEmpty) ...[
                      label('Text'),
                      ...texts.map(item),
                    ],
                    if (tables.isNotEmpty) ...[
                      label('Table'),
                      ...tables.map(item),
                    ],
                  ],
                ),
              ),
            ),
            Divider(height: WsMetrics.space3, color: ws.border),
            Container(
              height: SlashCommands._footerHeight,
              alignment: Alignment.centerLeft,
              padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
              child: Text(
                'Type to filter · ↑↓ · ⏎',
                style: TextStyle(
                  fontSize: WsMetrics.fsSmall,
                  color: ws.textFaint,
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
