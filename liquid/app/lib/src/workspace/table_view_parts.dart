import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import '../controls.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'anchored_menu.dart';
import 'explorer.dart' show untitled;

bool get touchFirst => switch (defaultTargetPlatform) {
  TargetPlatform.iOS || TargetPlatform.android => true,
  _ => false,
};

class TableNote extends StatelessWidget {
  const TableNote({super.key, required this.name, required this.onOpen});

  final String name;
  final VoidCallback onOpen;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Semantics(
      link: true,
      label: 'Open the Table ${name.isEmpty ? untitled : name}',
      child: MouseRegion(
        cursor: SystemMouseCursors.click,
        child: GestureDetector(
          key: const Key('table note'),
          behavior: HitTestBehavior.opaque,
          onTap: onOpen,
          child: ExcludeSemantics(
            child: Text.rich(
              TextSpan(
                text: 'Table: ',
                children: [
                  TextSpan(
                    text: name.isEmpty ? untitled : name,
                    style: TextStyle(color: ws.textMuted),
                  ),
                ],
              ),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                fontSize: WsMetrics.fsSmall,
                color: ws.textFaint,
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class SetupControl extends StatelessWidget {
  const SetupControl({
    super.key,
    required this.icon,
    required this.label,
    required this.value,
    required this.onTap,
  });

  final IconData icon;
  final String label;
  final String? value;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final value = this.value;
    return Semantics(
      button: true,
      label: value == null ? label : '$label: $value',
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        child: ExcludeSemantics(
          child: Container(
            height: 26,
            padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
            decoration: BoxDecoration(
              color: value == null ? null : ws.accentSubtle,
              borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
            ),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Icon(
                  icon,
                  size: 14,
                  color: value == null ? ws.textMuted : ws.accent,
                ),
                const SizedBox(width: WsMetrics.space2),
                Text(
                  value == null ? label : '$label:',
                  style: TextStyle(
                    fontSize: WsMetrics.fsCompact,
                    color: ws.textMuted,
                  ),
                ),
                if (value != null) ...[
                  const SizedBox(width: WsMetrics.space1),
                  Flexible(
                    child: Text(
                      value,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontSize: WsMetrics.fsCompact,
                        color: ws.text,
                      ),
                    ),
                  ),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class MoreMenu extends StatefulWidget {
  const MoreMenu({
    super.key,
    required this.label,
    required this.items,
    this.visible = true,
  });

  final String label;
  final List<(IconData, String, VoidCallback)> items;
  final bool visible;

  @override
  State<MoreMenu> createState() => _MoreMenuState();
}

class _MoreMenuState extends State<MoreMenu> {
  final _menu = OverlayPortalController();

  void _close() {
    if (mounted && _menu.isShowing) setState(_menu.hide);
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return AnchoredMenu(
      controller: _menu,
      end: true,
      height: 40.0 * widget.items.length + 8,
      onTapOutside: (_) => _close(),
      menu: (_) => Container(
        width: 220,
        padding: const EdgeInsets.all(WsMetrics.space1),
        decoration: BoxDecoration(
          color: ws.surface,
          borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
          boxShadow: ws.shadow,
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            for (final (icon, label, onTap) in widget.items)
              InkWell(
                borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                onTap: () {
                  _close();
                  onTap();
                },
                child: Container(
                  height: 32,
                  padding: const EdgeInsets.symmetric(
                    horizontal: WsMetrics.space3,
                  ),
                  child: Row(
                    children: [
                      Icon(icon, size: 14, color: ws.danger),
                      const SizedBox(width: WsMetrics.space3),
                      Flexible(
                        child: Text(
                          label,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(
                            fontSize: WsMetrics.fsUi,
                            color: ws.danger,
                          ),
                        ),
                      ),
                    ],
                  ),
                ),
              ),
          ],
        ),
      ),
      child: Opacity(
        opacity: widget.visible || _menu.isShowing ? 1 : 0,
        alwaysIncludeSemantics: true,
        child: Tooltip(
          message: widget.label,
          child: InkWell(
            key: Key(widget.label),
            borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
            onTap: () => setState(_menu.toggle),
            child: SizedBox(
              width: 28,
              height: 28,
              child: Icon(LucideIcons.ellipsis, size: 16, color: ws.textMuted),
            ),
          ),
        ),
      ),
    );
  }
}

enum LastView { view, table }

class LastViewDialog extends StatelessWidget {
  const LastViewDialog({super.key, required this.name, required this.rows});

  final String name;
  final int rows;

  @override
  Widget build(BuildContext context) {
    final named = name.isEmpty ? untitled : name;
    return ActionDialog(
      title: 'Delete the last View of “$named”?',
      actions: [
        PrimaryButton(
          label: 'Delete View',
          onPressed: () => Navigator.of(context).pop(LastView.view),
        ),
        SecondaryButton(
          label: 'Delete Table too',
          color: context.ws.danger,
          onPressed: () => Navigator.of(context).pop(LastView.table),
        ),
      ],
      children: [
        Text(
          'The Table and its ${rowCount(rows)} stay in Tables, unless you '
          'delete them too.',
          style: DangerDialog.text(context),
        ),
      ],
    );
  }
}

String rowCount(int rows) => '$rows ${rows == 1 ? 'row' : 'rows'}';

class TablePlaceholder extends StatelessWidget {
  const TablePlaceholder(this.text, {super.key});

  final String text;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      constraints: const BoxConstraints(minHeight: 72),
      margin: const EdgeInsets.only(top: WsMetrics.space2),
      padding: const EdgeInsets.all(WsMetrics.space4),
      alignment: Alignment.center,
      decoration: BoxDecoration(
        color: ws.subtle,
        borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
      ),
      child: Text(
        text,
        textAlign: TextAlign.center,
        style: TextStyle(fontSize: WsMetrics.fsCompact, color: ws.textMuted),
      ),
    );
  }
}
