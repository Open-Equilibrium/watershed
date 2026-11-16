import 'dart:async';

import 'package:flutter/material.dart';

import '../controls.dart';
import '../theme/tokens.g.dart';
import 'agent_hosts.dart';

/// Add Agent Host (ADR-0295): the address and pairing code another agent host shows for a device
/// (`meta invite --device`). Add pairs with it and saves it, and the dialog closes with the host;
/// Cancel closes it without a change.
class AddAgentHostDialog extends StatefulWidget {
  const AddAgentHostDialog({super.key, required this.hosts});

  final AgentHostsController hosts;

  @override
  State<AddAgentHostDialog> createState() => _AddAgentHostDialogState();
}

class _AddAgentHostDialogState extends State<AddAgentHostDialog> {
  final _address = TextEditingController();
  final _code = TextEditingController();
  String? _problem;
  bool _adding = false;

  @override
  void dispose() {
    _address.dispose();
    _code.dispose();
    super.dispose();
  }

  Future<void> _add() async {
    final address = _address.text.trim();
    final code = _code.text.trim();
    final listed = widget.hosts.value.hosts
        .where((host) => host.address == address)
        .firstOrNull;
    if (address.isEmpty || code.isEmpty) {
      setState(
        () => _problem = 'Enter the address and pairing code; nothing changed.',
      );
      return;
    }
    if (listed case final host?) {
      setState(
        () => _problem =
            'Could not add the agent host; nothing changed. ${host.name} is already listed.',
      );
      return;
    }
    setState(() {
      _adding = true;
      _problem = null;
    });
    try {
      final host = await widget.hosts.pair(address, code);
      if (mounted) Navigator.of(context).pop(host);
    } on AgentHostsError catch (error) {
      if (mounted) {
        setState(() {
          _adding = false;
          _problem = error.message;
        });
      }
    }
  }

  Widget _field(
    BuildContext context,
    String label,
    TextEditingController controller,
    String hint,
  ) => Column(
    crossAxisAlignment: CrossAxisAlignment.start,
    spacing: WsMetrics.space2,
    children: [
      Text(label, style: DangerDialog.text(context)),
      TextField(
        controller: controller,
        enabled: !_adding,
        autocorrect: false,
        enableSuggestions: false,
        decoration: InputDecoration(isDense: true, hintText: hint),
      ),
    ],
  );

  @override
  Widget build(BuildContext context) => ActionDialog(
    title: 'Add Agent Host',
    dismissible: !_adding,
    actions: [
      PrimaryButton(
        label: _adding ? 'Adding…' : 'Add agent host',
        onPressed: _adding ? null : () => unawaited(_add()),
      ),
    ],
    children: [
      Text(
        'On the other host, run meta remote on, then meta invite --device. Enter the address and pairing code it shows.',
        style: DangerDialog.text(context),
      ),
      _field(context, 'Host', _address, 'ws://host:3774'),
      _field(context, 'Pairing code', _code, 'XXXX-XXXX'),
      if (_problem case final problem?) WarningNotice(problem, danger: true),
    ],
  );
}
