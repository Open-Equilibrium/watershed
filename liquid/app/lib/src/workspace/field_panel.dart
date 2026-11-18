import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../rust/api/database.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';

String fieldKindName(BridgeFieldKind kind) => switch (kind) {
  BridgeFieldKind.text => 'Text',
  BridgeFieldKind.number => 'Number',
  BridgeFieldKind.select => 'Select',
  BridgeFieldKind.date => 'Date',
  BridgeFieldKind.checkbox => 'Checkbox',
  BridgeFieldKind.file => 'File',
};

IconData fieldKindIcon(BridgeFieldKind kind) => switch (kind) {
  BridgeFieldKind.text => LucideIcons.text,
  BridgeFieldKind.number => LucideIcons.hash,
  BridgeFieldKind.select => LucideIcons.circleChevronDown,
  BridgeFieldKind.date => LucideIcons.calendar,
  BridgeFieldKind.checkbox => LucideIcons.squareCheck,
  BridgeFieldKind.file => LucideIcons.paperclip,
};

class FieldPanel extends StatefulWidget {
  const FieldPanel({super.key, required this.onAdd, required this.onClose});

  final Future<void> Function(String name, BridgeFieldKind kind) onAdd;
  final VoidCallback onClose;

  @override
  State<FieldPanel> createState() => _FieldPanelState();
}

class _FieldPanelState extends State<FieldPanel> {
  final _name = TextEditingController();
  late final _focus = FocusNode(debugLabel: 'Field name', onKeyEvent: _key);
  var _kind = BridgeFieldKind.text;

  var _adding = false;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _focus.requestFocus();
    });
  }

  @override
  void dispose() {
    _name.dispose();
    _focus.dispose();
    super.dispose();
  }

  Future<void> _add(BridgeFieldKind kind) async {
    if (_adding) return;
    _adding = true;
    final name = _name.text.trim();
    try {
      await widget.onAdd(name.isEmpty ? fieldKindName(kind) : name, kind);
    } on Object catch (_) {
    } finally {
      _adding = false;
    }
  }

  KeyEventResult _key(FocusNode _, KeyEvent event) {
    if (event is KeyUpEvent || !_name.value.composing.isCollapsed) {
      return KeyEventResult.ignored;
    }
    if (event is KeyRepeatEvent &&
        (event.logicalKey == LogicalKeyboardKey.enter ||
            event.logicalKey == LogicalKeyboardKey.numpadEnter)) {
      return KeyEventResult.handled;
    }
    const kinds = BridgeFieldKind.values;
    switch (event.logicalKey) {
      case LogicalKeyboardKey.arrowDown || LogicalKeyboardKey.arrowUp:
        final by = event.logicalKey == LogicalKeyboardKey.arrowDown ? 1 : -1;
        setState(
          () => _kind = kinds[(kinds.indexOf(_kind) + by) % kinds.length],
        );
      case LogicalKeyboardKey.enter || LogicalKeyboardKey.numpadEnter:
        unawaited(_add(_kind));
      case LogicalKeyboardKey.escape:
        widget.onClose();
      default:
        return KeyEventResult.ignored;
    }
    return KeyEventResult.handled;
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      key: const Key('field panel'),
      width: 250,
      padding: const EdgeInsets.all(WsMetrics.space2),
      decoration: BoxDecoration(
        color: ws.surface,
        borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
        boxShadow: ws.shadow,
      ),
      child: DefaultTextStyle.merge(
        style: TextStyle(fontSize: WsMetrics.fsUi, color: ws.text),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Container(
              height: 30,
              margin: const EdgeInsets.fromLTRB(2, 2, 2, WsMetrics.space1),
              padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
              alignment: Alignment.centerLeft,
              decoration: BoxDecoration(
                color: ws.canvas,
                borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                border: Border.all(color: ws.accent, width: 1.5),
              ),
              child: TextField(
                controller: _name,
                focusNode: _focus,
                selectAllOnFocus: false,
                style: TextStyle(fontSize: WsMetrics.fsUi, color: ws.text),
                decoration: InputDecoration.collapsed(
                  hintText: 'Field name',
                  hintStyle: TextStyle(color: ws.textFaint),
                ),
              ),
            ),
            Padding(
              padding: const EdgeInsets.fromLTRB(
                WsMetrics.space3,
                WsMetrics.space2,
                WsMetrics.space3,
                2,
              ),
              child: Text(
                'Kind',
                style: TextStyle(
                  fontSize: WsMetrics.fsSmall,
                  color: ws.textFaint,
                ),
              ),
            ),
            for (final kind in BridgeFieldKind.values)
              MouseRegion(
                onEnter: (_) => setState(() => _kind = kind),
                child: Semantics(
                  button: true,
                  selected: kind == _kind,
                  child: GestureDetector(
                    onTap: () => unawaited(_add(kind)),
                    child: Container(
                      height: 30,
                      padding: const EdgeInsets.symmetric(
                        horizontal: WsMetrics.space3,
                      ),
                      decoration: BoxDecoration(
                        color: kind == _kind ? ws.hover : null,
                        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                      ),
                      child: Row(
                        children: [
                          Icon(
                            fieldKindIcon(kind),
                            size: 14,
                            color: ws.textFaint,
                          ),
                          const SizedBox(width: 10),
                          Text(fieldKindName(kind)),
                        ],
                      ),
                    ),
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }
}

class FieldPanelPortal extends StatelessWidget {
  const FieldPanelPortal({
    super.key,
    required this.panel,
    required this.plus,
    required this.onAdd,
    required this.child,
  });

  final OverlayPortalController panel;
  final LayerLink plus;

  final Future<void> Function(String name, BridgeFieldKind kind) onAdd;

  final Widget child;

  @override
  Widget build(BuildContext context) => OverlayPortal(
    controller: panel,
    overlayChildBuilder: (context) => Align(
      alignment: Alignment.topLeft,
      child: CompositedTransformFollower(
        link: plus,
        showWhenUnlinked: false,
        targetAnchor: Alignment.bottomRight,
        followerAnchor: Alignment.topRight,
        offset: const Offset(0, WsMetrics.space1),
        child: TapRegion(
          groupId: panel,
          onTapOutside: (_) => panel.hide(),
          child: TextFieldTapRegion(
            child: FieldPanel(onAdd: onAdd, onClose: panel.hide),
          ),
        ),
      ),
    ),
    child: child,
  );
}

class AddField extends StatelessWidget {
  const AddField({
    super.key,
    required this.plus,
    required this.panel,
    required this.onTap,
  });

  final LayerLink plus;
  final OverlayPortalController panel;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) => CompositedTransformTarget(
    link: plus,
    child: TapRegion(
      groupId: panel,
      child: Semantics(
        button: true,
        label: 'Add field',
        child: GestureDetector(
          key: const Key('add field'),
          behavior: HitTestBehavior.opaque,
          onTap: onTap,
          child: Center(
            child: Icon(
              LucideIcons.plus,
              size: 14,
              color: context.ws.textMuted,
            ),
          ),
        ),
      ),
    ),
  );
}
