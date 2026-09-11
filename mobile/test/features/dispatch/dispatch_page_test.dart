import 'dart:convert';

import 'package:buzz/features/dispatch/dispatch_page.dart';
import 'package:buzz/features/dispatch/dispatch_repository.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

const _actor =
    'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const _workspace = '2708da4e-e077-4b7a-9f1e-893cbe87b692';
const _issue = '69024291-26c5-430f-a021-e8bb130c2b01';

class _Repository extends Fake implements DispatchRepository {
  @override
  Future<String> savedDrafts(String workspace) async => '[]';
  @override
  String get relay => 'https://dispatch-fixture.invalid';
  @override
  String get pubkey => _actor;
  @override
  bool Function() get isCurrent =>
      () => true;
  Map<String, dynamic>? lastIntent;
  bool fail = false;
  Map<String, dynamic> issue = {
    'issueId': _issue,
    'identifier': 'D-69024291',
    'rootEvent': 'root',
    'tipEvent': 'tip',
    'title': 'Review the Summary release',
    'body': 'Confirm the installed behavior and record the evidence.',
    'status': 'in_review',
    'priority': 'P1',
    'assignee': _actor,
    'labels': ['release'],
    'externalRefs': [],
    'acceptedEvents': ['root', 'tip'],
  };
  final decision = {
    'event_id': 'decision-tip',
    'accepted_events': ['decision-tip'],
    'record': {
      'record_id': 'decision',
      'archived': false,
      'payload': {
        'type': 'decision',
        'issue_id': _issue,
        'question': 'Is this ready for release?',
        'context': 'The tests passed. Independent review is still pending.',
        'decider': _actor,
        'options': [
          {
            'id': 'wait',
            'label': 'Wait for review',
            'consequence':
                'Keep release paused until the independent check is complete.',
          },
          {
            'id': 'ship',
            'label': 'Release today',
            'consequence': 'Make the current build available now.',
          },
        ],
        'selected_option': null,
        'rationale': null,
        'evidence': [],
      },
    },
  };
  @override
  Future<List<DispatchWorkspace>> workspaces() async => [
    const DispatchWorkspace(_workspace, 'Buzz Development'),
  ];
  @override
  Future<PendingDispatchEdit?> pending(String workspace) async => null;
  Map<String, dynamic> data() => {
    'v': 1,
    'workspace': _workspace,
    'viewerRole': 'maintainer',
    'issues': [issue],
    'decisions': [decision],
    'admission': {'unsealed': 0},
    'diagnostics': [],
  };
  @override
  Future<Map<String, dynamic>> allOpen(String workspace) async => data();
  @override
  Future<Map<String, dynamic>> snapshot(
    String workspace, {
    String? issueId,
    String? after,
    bool includeClosed = false,
  }) async => data();
  @override
  Future<void> edit(
    String workspace,
    String issueId,
    Map<String, dynamic> intent, {
    Map<String, dynamic>? decisionPayload,
  }) async {
    lastIntent = intent;
    if (fail) {
      throw StateError('Connection interrupted. Your edit is saved.');
    }
    if (intent['operation'] == 'prepare_issue') {
      issue = {
        ...issue,
        'status': intent['set']['status'],
        'priority': intent['set']['priority'],
      };
    }
  }
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async {
    final manifest =
        jsonDecode(await rootBundle.loadString('FontManifest.json')) as List;
    for (final entry in manifest.cast<Map<String, dynamic>>()) {
      final family = entry['family'] as String;
      if (family == 'Inter' ||
          family == 'MaterialIcons' ||
          family.toLowerCase().contains('lucide')) {
        final loader = FontLoader(family);
        for (final font
            in (entry['fonts'] as List).cast<Map<String, dynamic>>()) {
          loader.addFont(rootBundle.load(font['asset'] as String));
        }
        await loader.load();
      }
    }
  });
  Future<_Repository> pump(
    WidgetTester tester, {
    double textScale = 1,
    bool dark = false,
  }) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final repository = _Repository();
    await tester.binding.setSurfaceSize(const Size(390, 844));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          dispatchRepositoryProvider.overrideWithValue(repository),
          savedPrefsProvider.overrideWithValue(prefs),
        ],
        child: MaterialApp(
          theme: dark ? AppTheme.dark() : AppTheme.light(),
          builder: (context, child) => MediaQuery(
            data: MediaQuery.of(
              context,
            ).copyWith(textScaler: TextScaler.linear(textScale)),
            child: child!,
          ),
          home: const RepaintBoundary(
            key: ValueKey('mobile-dispatch'),
            child: DispatchPage(),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    return repository;
  }

  testWidgets('mobile queue is readable at phone width', (tester) async {
    await pump(tester);
    expect(find.text('Review the Summary release'), findsOneWidget);
    expect(find.text('Your work'), findsOneWidget);
    expect(tester.takeException(), isNull);
    await expectLater(
      find.byKey(const ValueKey('mobile-dispatch')),
      matchesGoldenFile('goldens/queue.png'),
    );
  });
  testWidgets('issue metadata is readable in dark mode', (tester) async {
    await pump(tester, dark: true);
    await tester.tap(find.text('Review the Summary release'));
    await tester.pumpAndSettle();
    expect(find.text('In review'), findsOneWidget);
    expect(find.text('P1'), findsOneWidget);
    expect(find.text('release'), findsOneWidget);
    expect(tester.takeException(), isNull);
    await expectLater(
      find.byType(Scaffold),
      matchesGoldenFile('goldens/issue-dark.png'),
    );
  });
  testWidgets(
    'triage retains the exact base and preserves drafts after failure',
    (tester) async {
      final repository = await pump(tester);
      repository.fail = true;
      await tester.tap(find.text('Review the Summary release'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Triage this item'));
      await tester.pumpAndSettle();
      await tester.enterText(
        find.byType(TextField).last,
        'Waiting for an independent review',
      );
      await tester.ensureVisible(find.text('Save changes'));
      await tester.tap(find.text('Save changes'));
      await tester.pumpAndSettle();
      expect(repository.lastIntent?['expected_tip'], 'tip');
      expect(find.text('Waiting for an independent review'), findsOneWidget);
      expect(find.textContaining('Connection interrupted'), findsOneWidget);
      expect(tester.takeException(), isNull);
    },
  );
  testWidgets('decision presents consequences and requires a rationale', (
    tester,
  ) async {
    final repository = await pump(tester);
    await tester.tap(find.text('Decisions'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Is this ready for release?'));
    await tester.pumpAndSettle();
    await tester.ensureVisible(find.text('Review options and decide'));
    await tester.tap(find.text('Review options and decide'));
    await tester.pumpAndSettle();
    expect(find.text('Wait for review'), findsOneWidget);
    await tester.ensureVisible(find.text('Record decision'));
    await tester.tap(find.text('Record decision'));
    await tester.pumpAndSettle();
    expect(repository.lastIntent, isNull);
    expect(
      find.text('Choose an option and explain your decision.'),
      findsOneWidget,
    );
    expect(tester.takeException(), isNull);
  });
  testWidgets('larger text does not overflow the work queue', (tester) async {
    await pump(tester, textScale: 1.6);
    expect(tester.takeException(), isNull);
    await expectLater(
      find.byKey(const ValueKey('mobile-dispatch')),
      matchesGoldenFile('goldens/queue-large-text.png'),
    );
  });
  test('prepared issue bytes cannot silently change scope or extra fields', () {
    final request = {
      'operation': 'prepare_issue',
      'request_id': 'request',
      'issue_id': _issue,
      'expected_tip': 'tip',
      'reason': 'Ready',
      'set': {'status': 'todo'},
      'clear_assignee': false,
      'clear_priority': false,
    };
    final payload = {
      'request_id': 'request',
      'issue_id': _issue,
      'parent_event': 'tip',
      'reason': 'Ready',
      'set': {'status': 'todo'},
      'clear': [],
      'relations': {'add': [], 'remove': []},
      'external_refs': {'add': [], 'remove': []},
    };
    Map<String, dynamic> event(Map<String, dynamic> body) => {
      'pubkey': _actor,
      'kind': 40021,
      'tags': [
        ['h', _workspace],
      ],
      'content': jsonEncode(body),
    };
    expect(
      () => validatePreparedDispatchEvent(
        event(payload),
        request,
        _workspace,
        _actor,
      ),
      returnsNormally,
    );
    expect(
      () => validatePreparedDispatchEvent(
        {
          ...event(payload),
          'tags': [
            ['h', 'elsewhere'],
          ],
        },
        request,
        _workspace,
        _actor,
      ),
      throwsFormatException,
    );
    expect(
      () => validatePreparedDispatchEvent(
        event({
          ...payload,
          'set': {'status': 'todo', 'title': 'unrequested title'},
        }),
        request,
        _workspace,
        _actor,
      ),
      throwsFormatException,
    );
  });
  test(
    'decision preparation preserves reviewed evidence regardless of object key order',
    () {
      final baseline = {
        'type': 'decision',
        'decider': _actor,
        'question': 'Ship?',
        'options': [
          {'id': 'wait', 'label': 'Wait'},
        ],
        'selected_option': null,
        'rationale': null,
      };
      final request = {
        'operation': 'prepare_decision',
        'request_id': 'request',
        'record_id': 'record',
        'expected_event': 'tip',
        'option': 'wait',
        'rationale': 'Review pending',
        '_baseline_payload': baseline,
      };
      final chosen = {
        ...baseline,
        'selected_option': 'wait',
        'rationale': 'Review pending',
      };
      Map<String, dynamic> event(Map<String, dynamic> payload) => {
        'pubkey': _actor,
        'kind': 40026,
        'tags': [
          ['h', _workspace],
        ],
        'content': jsonEncode({
          'request_id': 'request',
          'record_id': 'record',
          'parent_event': 'tip',
          'payload': payload,
        }),
      };
      expect(
        () => validatePreparedDispatchEvent(
          event({
            for (final key in chosen.keys.toList().reversed) key: chosen[key],
          }),
          request,
          _workspace,
          _actor,
        ),
        returnsNormally,
      );
      expect(
        () => validatePreparedDispatchEvent(
          event({...chosen, 'question': 'Different question'}),
          request,
          _workspace,
          _actor,
        ),
        throwsFormatException,
      );
    },
  );
}
