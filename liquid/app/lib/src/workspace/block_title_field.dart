import 'package:flutter/material.dart';

import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'explorer.dart' show untitled;

class BlockTitleField extends StatelessWidget {
  const BlockTitleField({
    super.key,
    required this.controller,
    required this.focusNode,
    this.selectAllOnFocus,
  });

  final TextEditingController controller;
  final FocusNode focusNode;
  final bool? selectAllOnFocus;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return TextField(
      controller: controller,
      focusNode: focusNode,
      selectAllOnFocus: selectAllOnFocus,
      style: TextStyle(
        fontSize: WsMetrics.fsProse,
        fontWeight: WsMetrics.fwSemibold,
        color: ws.text,
      ),
      decoration: InputDecoration.collapsed(
        hintText: untitled,
        hintStyle: TextStyle(color: ws.textFaint),
      ),
    );
  }
}
