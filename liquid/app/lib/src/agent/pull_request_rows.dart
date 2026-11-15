import 'dart:async';

import 'package:flutter/material.dart';

import '../rust/api/git.dart';
import '../session/session_state.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'session_card_rows.dart';
import 'session_menus.dart';

/// The checkout's pull request, as T3 Code's thread details show it: its state, its number and
/// title. A click opens it in the browser.
class PullRequestRows extends StatelessWidget {
  const PullRequestRows({super.key, required this.state});

  final SessionState state;

  @override
  Widget build(BuildContext context) => ValueListenableBuilder<GitState>(
    valueListenable: state.git,
    builder: (context, git, _) => ValueListenableBuilder<BridgeGitStatus?>(
      valueListenable: git.status,
      builder: (context, status, _) {
        final pr = status?.pullRequest;
        if (status == null || pr == null) return const SizedBox.shrink();
        final presentation = _presentation(pr, context.ws);
        final terms = terminologyFor(status);
        return Tooltip(
          message:
              '${terms.short} #${pr.number} - ${presentation.label}: ${pr.title}',
          child: CardRow(
            onTap: () => unawaited(openInBrowser(state, pr.url)),
            children: [
              Icon(presentation.icon, size: 16, color: presentation.tone),
              Flexible(
                child: Text(
                  '#${pr.number}: ${pr.title}',
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
              Text(
                presentation.label,
                style: TextStyle(
                  color: presentation.tone,
                  fontSize: WsMetrics.fsSmall,
                  fontWeight: WsMetrics.fwRegular,
                ),
              ),
            ],
          ),
        );
      },
    ),
  );
}

/// The label, icon and tone of a pull request's state; an open draft reads as Draft.
({String label, IconData icon, Color tone}) _presentation(
  BridgeBranchPullRequest pr,
  WsPalette ws,
) {
  if (pr.state == 'open' && pr.draft) {
    return (
      label: 'Draft',
      icon: LucideIcons.gitPullRequest,
      tone: ws.textMuted,
    );
  }
  return switch (pr.state) {
    'open' => (
      label: 'Open',
      icon: LucideIcons.gitPullRequest,
      tone: ws.success,
    ),
    'closed' => (label: 'Closed', icon: LucideIcons.circleX, tone: ws.danger),
    _ => (label: 'Merged', icon: LucideIcons.checkCircle, tone: ws.accent),
  };
}
