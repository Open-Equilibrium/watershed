import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/format.dart';
import 'package:liquid/src/rust/api/history.dart';
import 'package:liquid/src/rust/api/session.dart';
import 'package:liquid/src/trajectory/context_track.dart';
import 'package:liquid/src/trajectory/trajectory_model.dart';

import 'fake_backend.dart';

void main() {
  group('session graph', () {
    // e1 prompt → e2 response → p1 approval → e3 tool result → e4 response (tip A)
    //                         ↘ b1 prompt → b2 response (tip B)
    //                         ↘ c1 prompt (tip C, labelled)
    final tree = [
      node('e0', null, BridgeEntryKind.other),
      node(
        'e1',
        'e0',
        BridgeEntryKind.user,
        preview: 'Add rate limiting',
        at: 1000,
      ),
      node('t1', 'e1', BridgeEntryKind.custom),
      node('e2', 't1', BridgeEntryKind.assistant, preview: 'I will look.'),
      node('p1', 'e2', BridgeEntryKind.approval, preview: 'bash'),
      node('e3', 'p1', BridgeEntryKind.toolResult, preview: 'bash'),
      node('e4', 'e3', BridgeEntryKind.assistant, preview: 'Done.', at: 26900),
      node('b1', 'e2', BridgeEntryKind.user, preview: 'Use a token bucket'),
      node('b2', 'b1', BridgeEntryKind.assistant, preview: 'Ok.', at: 19500),
      node('c1', 'e2', BridgeEntryKind.user, preview: 'Fixed', label: 'fixed'),
    ];

    test('draws one dot per step and one lane per branch', () {
      final graph = sessionGraph(tree, 'b2');
      expect(
        [
          for (final row in graph.rows)
            (row.node.id, row.lane, row.parentRow, row.step),
        ],
        [
          ('e1', 0, null, 1),
          ('e2', 0, 0, 2),
          ('p1', 0, 1, 3),
          ('e3', 0, 2, 4),
          ('e4', 0, 3, 5),
          ('b1', 1, 1, 3),
          ('b2', 1, 5, 4),
          ('c1', 2, 1, 3),
        ],
        reason: 'the step numbers match the ledger of each branch',
      );
      expect(graph.lanes, 3);
      expect([graph.rows[0].turn, graph.rows[5].turn], [1, 2]);
    });

    test(
      'names branches by label or latest prompt, timed from the first prompt',
      () {
        final graph = sessionGraph(tree, 'b2');
        expect(
          [
            for (final branch in graph.branches)
              (branch.ref, branch.tip, branch.name, branch.durationMs),
          ],
          [
            ('A', 'e4', 'Add rate limiting', 25900),
            ('B', 'b2', 'Use a token bucket', 18500),
            ('C', 'c1', 'fixed', null),
          ],
        );
        expect(
          graph.branchOf('e2')?.ref,
          'A',
          reason: 'follows first children',
        );
        expect(graph.branchOf('b1')?.ref, 'B');
      },
    );

    test('connects branches that fork before the first prompt', () {
      final graph = sessionGraph([
        node('e0', null, BridgeEntryKind.other),
        node('e1', 'e0', BridgeEntryKind.user, preview: 'First'),
        node('x1', 'e0', BridgeEntryKind.user, preview: 'Instead'),
      ], 'x1');
      expect(
        [
          for (final row in graph.rows)
            (row.node.id, row.lane, row.parentRow, row.step),
        ],
        [('e0', 0, null, 0), ('e1', 0, 0, 1), ('x1', 1, 0, 1)],
      );
    });

    test('marks a leaf with children as the start of the next branch', () {
      final graph = sessionGraph(tree, 'e3');
      final row = graph.rows.singleWhere((row) => row.node.id == 'e3');
      expect((row.pending, row.branch), (true, null));
      expect(graph.branchOf('e3')?.ref, 'A');
    });
  });

  test('marks where the current branch leaves the others', () {
    // e1 → e2 → e3 → e4 (A);  e2 → b1 → b2 (B)
    final tree = [
      node('e1', null, BridgeEntryKind.user, preview: 'Go'),
      node('e2', 'e1', BridgeEntryKind.assistant),
      node('e3', 'e2', BridgeEntryKind.toolResult, preview: 'read'),
      node('e4', 'e3', BridgeEntryKind.assistant),
      node('b1', 'e2', BridgeEntryKind.user, preview: 'Instead'),
      node('b2', 'b1', BridgeEntryKind.assistant),
    ];
    final marker = branchMarker(tree, 'b2')!;
    expect(
      (marker.fork, marker.ref, marker.step, marker.other, marker.otherMore),
      ('e2', 'B', 2, 'A', 2),
    );
    final pending = branchMarker(tree, 'e2')!;
    expect(
      (pending.fork, pending.ref, pending.step, pending.other),
      ('e2', null, 2, 'A'),
    );
    expect(branchMarker(tree.take(4).toList(), 'e4'), isNull);
  });

  test('groups records into turns with their tool counts', () {
    final records = [
      record('e1', 1, BridgeRecordKind.user, 'Add rate limiting', 0),
      record('e2', 1, BridgeRecordKind.message, 'Look', 0, duration: 2100),
      record('e3', 1, BridgeRecordKind.tool, 'read', 2200, duration: 18),
      record('e4', 1, BridgeRecordKind.tool, 'edit', 2300, duration: 9),
      record('e5', 1, BridgeRecordKind.tool, 'read', 2400, duration: 9),
      record('e6', 2, BridgeRecordKind.user, 'Again', 9000),
    ];
    expect(
      [
        for (final turn in turnGroups(records))
          (turn.turn, turn.prompt, turn.first, turn.last, turn.durationMs),
      ],
      [(1, 'Add rate limiting', 0, 4, 2409), (2, 'Again', 5, 5, 0)],
    );
    expect(turnGroups(records).first.tools, {'read': 2, 'edit': 1});
  });

  group('context track', () {
    BridgeStepContext request(int prompt, int tools, int messages) =>
        BridgeStepContext(
          promptChars: prompt,
          sections: const ['preamble', 'project_context'],
          tools: const ['bash'],
          toolChars: tools,
          messages: 1,
          messageChars: messages,
          newSincePrevious: const [],
        );
    // A prompt, request 1 (1000 tokens for 4000 characters), an approval, a tool result,
    // request 2 (3100 tokens for 12400 characters) and a prompt no request read yet.
    final records = [
      record('u1', 1, BridgeRecordKind.user, 'Fix it', 0, added: 400),
      record(
        'm1',
        1,
        BridgeRecordKind.message,
        'Reading',
        10,
        usage: usage(1000, 50, 0),
        context: request(2000, 1600, 400),
        added: 200,
      ),
      record('p1', 1, BridgeRecordKind.approval, 'bash', 20),
      record('t1', 1, BridgeRecordKind.tool, 'bash', 30, added: 8000),
      record(
        'm2',
        1,
        BridgeRecordKind.message,
        'Done',
        40,
        usage: usage(3100, 20, 0),
        context: request(2000, 1600, 8800),
        added: 80,
      ),
      record('u2', 2, BridgeRecordKind.user, 'Again', 50, added: 40),
    ];

    test('measures the context at each request and estimates between', () {
      final track = contextTrack(records);
      expect(
        (track.start.tokens, track.start.exact),
        (900, false),
        reason: 'the system prompt and tools of the first request',
      );
      expect(
        [
          for (final level in track.levels)
            (level.tokens, level.exact, level.added, level.addedExact),
        ],
        [
          // The last step before a request takes the request's measured size.
          (1000, true, 100, false),
          // A response adds its output to the request it answered.
          (1050, false, 50, true),
          (1050, false, 0, true),
          (3100, true, 2000, false),
          (3120, false, 20, true),
          (3130, false, 10, false),
        ],
      );
      expect(
        [
          for (final turn in turnGroups(records))
            (track.before(turn).tokens, track.levels[turn.last].tokens),
        ],
        [(900, 3120), (3120, 3130)],
      );
    });

    test('estimates a step at the ratio of the request that read it', () {
      // Request 1 reads 1 token per 4 characters, request 2 1 per 2; the tool result between
      // them is estimated as request 2 reads it, as the inspector splits that request.
      final track = contextTrack([
        record('u1', 1, BridgeRecordKind.user, 'Fix it', 0, added: 400),
        record(
          'm1',
          1,
          BridgeRecordKind.message,
          'Reading',
          10,
          usage: usage(1000, 50, 0),
          context: request(2000, 1600, 400),
        ),
        record('t1', 1, BridgeRecordKind.tool, 'bash', 30, added: 8000),
        record('t2', 1, BridgeRecordKind.tool, 'read', 40, added: 2000),
        record(
          'm2',
          1,
          BridgeRecordKind.message,
          'Done',
          50,
          usage: usage(6200, 20, 0),
          context: request(2000, 1600, 8800),
        ),
      ]);
      expect(
        [for (final level in track.levels) (level.tokens, level.added)],
        [(1000, 100), (1050, 50), (5050, 4000), (6200, 1000), (6220, 20)],
      );
    });

    test('names the request that read each step', () {
      expect(
        [for (var i = 0; i < records.length; i++) requestOf(records, i)],
        [1, 1, 4, 4, 4, null],
        reason:
            'a response is its own request; other steps go into the next one',
      );
      expect(requestNumber(records, 4), 2);
    });

    test('splits a request into its parts', () {
      final parts = requestParts(
        records[4],
        detail: const BridgeContextDetail(
          sections: [
            BridgeContextPart(name: 'preamble', chars: 1200),
            BridgeContextPart(name: 'project_context', chars: 800),
          ],
          tools: [BridgeContextPart(name: 'bash', chars: 1600)],
          messages: [
            BridgeContextMessage(
              entry: 'u1',
              label: 'prompt · Fix it',
              chars: 400,
              inContext: true,
              fresh: false,
            ),
            BridgeContextMessage(
              entry: 'old',
              label: 'response · summarized',
              chars: 9000,
              inContext: false,
              fresh: false,
            ),
            BridgeContextMessage(
              entry: 't1',
              label: 'result of bash',
              chars: 8000,
              inContext: true,
              fresh: true,
            ),
          ],
        ),
        stepChars: 8000,
      );
      expect(
        (
          parts.system,
          parts.projectContext,
          parts.tools,
          parts.step,
          parts.carried,
        ),
        (300, 200, 400, 2000, 200),
        reason: 'shares of the measured 3100 tokens; the rest was carried over',
      );
      expect(parts.largest, [
        ('result of bash', 2000),
        ('Tools', 400),
        ('System prompt', 300),
      ], reason: 'a message the model no longer saw does not count');
      expect(
        requestParts(records[4], stepChars: 8000).projectContext,
        0,
        reason:
            'without the detail the system prompt keeps its project context',
      );
    });

    test('finds the window and where pi compacts', () {
      const models = [
        BridgeModel(
          provider: 'faux',
          id: 'faux-1',
          name: 'Faux One',
          contextWindow: 200000,
        ),
      ];
      expect(contextWindowOf('faux-1', models), 200000);
      expect(contextWindowOf('faux-9', models), isNull);
      expect(compactionPoint(200000), 183616);
      expect(compactionPoint(16000), isNull);
    });
  });

  group('overview', () {
    final records = [
      record('e1', 1, BridgeRecordKind.user, 'Go', 0),
      record(
        'e2',
        1,
        BridgeRecordKind.message,
        'Run',
        100,
        duration: 3400,
        firstToken: 900,
      ),
      record('a1', 1, BridgeRecordKind.approval, 'bash', 3600, duration: 9400),
      record('e3', 1, BridgeRecordKind.tool, 'bash', 13000, duration: 6800),
    ];

    test('places steps on the real time axis or in equal slots', () {
      expect(
        [
          for (final segment in segments(records, equalWidth: false))
            (segment.lane, segment.start, segment.end, segment.firstToken),
        ],
        [
          (OverviewLane.model, 100.0, 3500.0, 1000.0),
          (OverviewLane.you, 3600.0, 13000.0, null),
          (OverviewLane.tools, 13000.0, 19800.0, null),
        ],
      );
      expect(
        [
          for (final segment in segments(records, equalWidth: true))
            (segment.index, segment.start, segment.end),
        ],
        [(1, 1.0, 2.0), (2, 2.0, 3.0), (3, 3.0, 4.0)],
      );
      expect(span(records, equalWidth: false), 19800);
      expect(span(records, equalWidth: true), 4);
    });

    test('finds the steps a focused range overlaps', () {
      expect(inRange(records, (3000, 5000), equalWidth: false), [
        false,
        true,
        true,
        false,
      ]);
      expect(inRange(records, (0.5, 1.5), equalWidth: true), [
        true,
        true,
        false,
        false,
      ]);
    });
  });

  test('aligns steps of the same kind after the shared steps', () {
    final shared = [
      record('e1', 1, BridgeRecordKind.user, 'Go', 0),
      record('e2', 1, BridgeRecordKind.message, 'Look', 100, duration: 900),
    ];
    final a = [
      ...shared,
      record('a1', 1, BridgeRecordKind.message, 'Sliding window', 1500),
      record('a2', 1, BridgeRecordKind.tool, 'edit', 2000),
      record('a3', 1, BridgeRecordKind.tool, 'write', 2100),
      record('a4', 1, BridgeRecordKind.tool, 'bash', 2200),
    ];
    final b = [
      ...shared,
      record('b0', 2, BridgeRecordKind.user, 'Use a bucket', 60000),
      record('b1', 2, BridgeRecordKind.message, 'Token bucket', 60100),
      record('b2', 2, BridgeRecordKind.tool, 'write', 61000),
      record('b3', 2, BridgeRecordKind.tool, 'bash', 61100),
    ];
    final comparison = compareBranches(a, b);
    expect(comparison.shared, 2);
    expect(
      [
        for (final (left, right) in comparison.rows)
          (left?.entry, right?.entry),
      ],
      [(null, 'b0'), ('a1', 'b1'), ('a2', null), ('a3', 'b2'), ('a4', 'b3')],
      reason: 'same tools pair up; a step without counterpart stands alone',
    );
    expect(
      (comparison.idleA, comparison.idleB),
      (0, 59000),
      reason: 'only the wait for a new prompt counts as idle',
    );
  });

  test('describes tool outcomes, shares and changed files', () {
    expect(
      [
        outcomeText(const BridgeToolOutcome(added: 9, removed: 2)),
        outcomeText(const BridgeToolOutcome(added: 38)),
        outcomeText(
          const BridgeToolOutcome(exitCode: 0, testsPassed: 14, testsFailed: 0),
        ),
        outcomeText(
          const BridgeToolOutcome(exitCode: 1, testsPassed: 12, testsFailed: 2),
        ),
        outcomeText(const BridgeToolOutcome()),
      ],
      ['+9 −2', '+38', 'exit 0 · 14 ✓', 'exit 1 · 12 ✓ 2 ✗', null],
    );
    expect(shares(6140, [2310, 1050, 2780]), [2310, 1050, 2780]);
    expect(shares(100, [1, 1, 2]), [25, 25, 50]);
    expect(shares(100, [0, 0, 0]), [0, 0, 0]);
    final edits = [
      record(
        'e1',
        1,
        BridgeRecordKind.tool,
        'edit',
        0,
        target: 'src/login.ts',
        changedFile: 'src/login.ts',
      ),
      record(
        'e2',
        1,
        BridgeRecordKind.tool,
        'write',
        0,
        target: 'test/login.test.ts',
        changedFile: 'test/login.test.ts',
      ),
      record(
        'e3',
        1,
        BridgeRecordKind.tool,
        'edit',
        0,
        target: 'src/login.ts',
        changedFile: 'src/login.ts',
      ),
      record(
        'e4',
        1,
        BridgeRecordKind.tool,
        'edit',
        0,
        target: 'README.md',
        failed: true,
      ),
      record(
        'e5',
        1,
        BridgeRecordKind.tool,
        'bash',
        0,
        outcome: const BridgeToolOutcome(testsPassed: 3, testsFailed: 0),
      ),
      record(
        'e6',
        1,
        BridgeRecordKind.tool,
        'bash',
        0,
        outcome: const BridgeToolOutcome(testsPassed: 14, testsFailed: 1),
      ),
    ];
    expect(changedFiles(edits), {'src/login.ts', 'test/login.test.ts'});
    expect(
      [isTestPath('test/login.test.ts'), isTestPath('src/login.ts')],
      [true, false],
    );
    expect(lastTests(edits)?.testsPassed, 14);
  });

  test('formats durations, counts and tool arguments', () {
    expect(
      [formatDuration(18), formatDuration(3400), formatDuration(125000)],
      ['18 ms', '3.4 s', '2m 05s'],
    );
    expect(
      [formatDuration(59949), formatDuration(59950), formatDuration(60000)],
      ['59.9 s', '1m 00s', '1m 00s'],
      reason: 'a minute once seconds round to 60',
    );
    expect([formatCost(20000), formatCost(1000)], ['\$0.02', '\$0.001']);
    expect(
      [formatCount(6140), formatCompact(6140), formatCompact(820)],
      ['6,140', '6.1k', '820'],
    );
    expect(
      [
        toolSummary(
          record(
            'e1',
            1,
            BridgeRecordKind.tool,
            'bash',
            0,
            target: 'npm test -- login',
          ),
        ),
        toolSummary(record('e2', 1, BridgeRecordKind.tool, 'read', 0)),
      ],
      ['bash · npm test -- login', 'read'],
    );
  });
}

BridgeTreeNode node(
  String id,
  String? parent,
  BridgeEntryKind kind, {
  String preview = '',
  String? label,
  int? at,
}) => BridgeTreeNode(
  failed: false,
  id: id,
  parent: parent,
  kind: kind,
  preview: preview,
  label: label,
  atMs: at,
);
