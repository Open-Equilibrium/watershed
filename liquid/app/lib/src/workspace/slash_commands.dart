import 'package:flutter/material.dart';

import '../rust/api/database.dart';
import '../rust/api/workspace.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'page_editor.dart' show codeShortcut, markdownShortcuts;

@immutable
class SlashCommand {
  const SlashCommand(
    this.label,
    this.icon, {
    this.style,
    this.block,
    this.description,
    this.existing = false,
    this.layout = BridgeLayout.table,
  });

  final bool existing;

  final BridgeLayout layout;

  final String label;
  final IconData icon;
  final BridgeTextStyle? style;

  final BridgeBlockKind? block;

  final String? description;

  String? get shortcut => block == BridgeBlockKind.code
      ? codeShortcut
      : markdownShortcuts.entries
            .where((shortcut) => style != null && shortcut.value == style)
            .firstOrNull
            ?.key
            .trimRight();
}

const slashCommands = [
  SlashCommand('Text', LucideIcons.text, style: BridgeTextStyle.paragraph),
  SlashCommand(
    'Heading 1',
    LucideIcons.heading1,
    style: BridgeTextStyle.heading1,
  ),
  SlashCommand(
    'Heading 2',
    LucideIcons.heading2,
    style: BridgeTextStyle.heading2,
  ),
  SlashCommand(
    'Heading 3',
    LucideIcons.heading3,
    style: BridgeTextStyle.heading3,
  ),
  SlashCommand(
    'Bulleted list',
    LucideIcons.list,
    style: BridgeTextStyle.bullet,
  ),
  SlashCommand(
    'Numbered list',
    LucideIcons.listOrdered,
    style: BridgeTextStyle.numbered,
  ),
  SlashCommand(
    'To-do list',
    LucideIcons.squareCheck,
    style: BridgeTextStyle.todo,
  ),
  SlashCommand('Quote', LucideIcons.textQuote, style: BridgeTextStyle.quote),
  SlashCommand(
    'Code',
    LucideIcons.code,
    block: BridgeBlockKind.code,
    description: 'Code with syntax colors',
  ),
  SlashCommand(
    'Media',
    LucideIcons.image,
    block: BridgeBlockKind.media,
    description: 'An image or a file',
  ),
  SlashCommand(
    'Table',
    LucideIcons.table,
    block: BridgeBlockKind.database,
    description: 'New fields and rows',
  ),
  SlashCommand(
    'Board',
    LucideIcons.kanban,
    block: BridgeBlockKind.database,
    description: 'New Table as a Board',
    layout: BridgeLayout.board,
  ),
  SlashCommand(
    'Calendar',
    LucideIcons.calendar,
    block: BridgeBlockKind.database,
    description: 'New Table as a Calendar',
    layout: BridgeLayout.calendar,
  ),
  SlashCommand(
    'Existing Table…',
    LucideIcons.table,
    block: BridgeBlockKind.database,
    description: 'Another View of a Table',
    existing: true,
  ),
  SlashCommand(
    'Chart',
    LucideIcons.barChart3,
    block: BridgeBlockKind.chart,
    description: 'Draws data from a Table or a Calculation',
  ),
  SlashCommand(
    'Calculation',
    LucideIcons.calculator,
    block: BridgeBlockKind.calculation,
    description: 'Labeled values, from one line to a report',
  ),
  SlashCommand(
    'Whiteboard',
    LucideIcons.presentation,
    block: BridgeBlockKind.whiteboard,
    description: 'Draw, and place shapes and text',
  ),
];

List<SlashCommand> slashCommandsFor(String query) => [
  for (final choice in slashCommands)
    if (choice.label.toLowerCase().contains(query.toLowerCase())) choice,
];

class SlashCommands extends StatefulWidget {
  const SlashCommands({
    super.key,
    required this.choices,
    required this.chosen,
    required this.onChoose,
    required this.onHover,
  });

  final List<SlashCommand> choices;

  final int chosen;
  final ValueChanged<SlashCommand> onChoose;
  final ValueChanged<int> onHover;

  static const _labelHeight = 26.0;
  static const _footerHeight = 26.0;

  static double _itemHeight(SlashCommand choice) =>
      choice.description == null ? 36 : 40;

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

  void _reveal() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted || !_list.hasClients) return;
      final choices = widget.choices;
      final index = widget.chosen;
      if (index >= choices.length) return;
      final choice = choices[index];
      final block = choice.style == null;
      final labels = block && choices.first.style != null ? 2 : 1;
      final item =
          labels * SlashCommands._labelHeight +
          choices
              .take(index)
              .fold(
                0.0,
                (top, above) => top + SlashCommands._itemHeight(above),
              );
      final first = index == 0 || (choices[index - 1].style == null) != block;
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
    final blocks = choices.where((choice) => choice.style == null);
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
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
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
                    if (blocks.isNotEmpty) ...[
                      label('Blocks'),
                      ...blocks.map(item),
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
