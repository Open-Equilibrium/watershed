import 'dart:async';

import 'package:flutter/material.dart';

import '../controls.dart';
import '../session/session_state.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';

Future<void> showPublishDialog(BuildContext context, SessionState state) =>
    showDialog<void>(
      context: context,
      builder: (_) => _PublishDialog(state: state),
    );

class _PublishDialog extends StatefulWidget {
  const _PublishDialog({required this.state});

  final SessionState state;

  @override
  State<_PublishDialog> createState() => _PublishDialogState();
}

class _PublishDialogState extends State<_PublishDialog> {
  final _repository = TextEditingController();
  bool _public = false;
  bool _publishing = false;
  bool _published = false;
  String? _error;

  @override
  void initState() {
    super.initState();
    _repository.addListener(() => setState(() {}));
  }

  @override
  void dispose() {
    _repository.dispose();
    super.dispose();
  }

  bool get _named {
    final parts = _repository.text.trim().split('/');
    return parts.length >= 2 &&
        parts.first.trim().isNotEmpty &&
        parts.skip(1).join('/').trim().isNotEmpty;
  }

  Future<void> _publish() async {
    final repository = _repository.text.trim();
    setState(() {
      _publishing = true;
      _error = null;
    });
    try {
      await widget.state.git.value.backend.publish(
        'github',
        repository,
        public: _public,
      );
      if (mounted) {
        setState(() {
          _publishing = false;
          _published = true;
        });
      }
    } on Object catch (error) {
      if (!mounted) return;
      setState(() {
        _publishing = false;
        _error = 'Publish failed. $error';
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final published = _published;
    return ActionDialog(
      title: 'Publish repository',
      dismissible: !_publishing,
      dismiss: published ? 'Done' : null,
      actions: published
          ? const []
          : [
              PrimaryButton(
                label: _publishing ? 'Publishing...' : 'Publish',
                onPressed: _named && !_publishing
                    ? () => unawaited(_publish())
                    : null,
              ),
            ],
      children: published
          ? [
              Row(
                spacing: WsMetrics.space3,
                children: [
                  Icon(LucideIcons.check, size: 16, color: ws.success),
                  _label('Repository published', ws),
                ],
              ),
              Text(_repository.text.trim(), style: monoStyle(context)),
            ]
          : [
              Text(
                'Pick where to host it, then point us at a repo to push to.',
                style: DangerDialog.text(context),
              ),
              _label('Repository', ws),
              TextField(
                controller: _repository,
                autofocus: true,
                enabled: !_publishing,
                style: monoStyle(context),
                onSubmitted: (_) {
                  if (_named && !_publishing) unawaited(_publish());
                },
                decoration: InputDecoration(
                  isDense: true,
                  prefixText: 'github.com/',
                  prefixStyle: monoStyle(context, color: ws.textMuted),
                  hintText: 'owner/repo',
                ),
              ),
              _label('Visibility', ws),
              Segments<bool>(
                options: const [(false, 'Private'), (true, 'Public')],
                selected: _public,
                onSelect: (value) {
                  if (!_publishing) setState(() => _public = value);
                },
              ),
              Text(
                _public ? 'Anyone on the web' : 'Only invited people',
                style: DangerDialog.text(context),
              ),
              if (_publishing)
                const WarningNotice(
                  'Publishing repository to GitHub...',
                  info: true,
                ),
              if (_error case final error?) WarningNotice(error, danger: true),
            ],
    );
  }
}

Widget _label(String text, WsPalette ws) => Text(
  text,
  style: TextStyle(
    fontSize: WsMetrics.fsCompact,
    fontWeight: WsMetrics.fwMedium,
    color: ws.text,
  ),
);
