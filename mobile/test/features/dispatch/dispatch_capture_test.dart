import 'dart:convert';
import 'package:buzz/shared/dispatch/dispatch_capture.dart';
import 'package:buzz/shared/dispatch/dispatch_capture_sheet.dart';
import 'package:buzz/shared/dispatch/dispatch_repository.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

const channel = '11111111-1111-4111-8111-111111111111';
final source = 'a' * 64;

class Repository extends Fake implements DispatchRepository {
  final List<List<String>> captures = [];
  bool fail = false;
  @override
  Future<void> capture(String channel, String source, String title) async {
    if (fail) throw StateError('Capture not accepted');
    captures.add([channel, source, title]);
  }
}

void main() {
  test(
    'capture uses SDK request identity, same-channel triage and source tags',
    () {
      final root = dispatchCaptureRoot(channel, source, 'Follow up');
      expect(root['request_id'], '4155f154-f259-5902-bb4d-9c9f14baf4f6');
      expect(root['issue_id'], root['request_id']);
      expect(root['home_channel_id'], channel);
      expect(root['status'], 'triage');
      expect(root['body'], '');
      expect(
        dispatchCaptureTags(channel, root),
        contains(equals(['e', source, '', 'source'])),
      );
      final event = {
        'kind': 40020,
        'pubkey': 'actor',
        'content': jsonEncode(root),
        'tags': dispatchCaptureTags(channel, root),
      };
      final request = {
        'operation': 'capture',
        'request_id': root['request_id'],
        'root': root,
      };
      expect(
        () => validatePreparedDispatchEvent(event, request, channel, 'actor'),
        returnsNormally,
      );
      final changed = {...root, 'status': 'todo'};
      expect(
        () => validatePreparedDispatchEvent(
          {...event, 'content': jsonEncode(changed)},
          request,
          channel,
          'actor',
        ),
        throwsFormatException,
      );
      expect(
        () => dispatchCaptureRoot(channel, 'pending-message', 'Follow up'),
        throwsFormatException,
      );
    },
  );
  testWidgets(
    'capture sheet reviews title and preserves failed intent before success',
    (tester) async {
      final repository = Repository()..fail = true;
      await tester.pumpWidget(
        ProviderScope(
          overrides: [dispatchRepositoryProvider.overrideWithValue(repository)],
          child: MaterialApp(
            theme: AppTheme.light(),
            home: Scaffold(
              body: DispatchCaptureSheet(
                channel: channel,
                source: source,
                content: 'First line\nPrivate source details',
              ),
            ),
          ),
        ),
      );
      expect(find.text('First line'), findsOneWidget);
      await tester.tap(find.text('Create linked issue'));
      await tester.pumpAndSettle();
      expect(find.textContaining('Capture not accepted'), findsOneWidget);
      expect(find.text('First line'), findsOneWidget);
      expect(repository.captures, isEmpty);
      repository.fail = false;
      await tester.tap(find.text('Create linked issue'));
      await tester.pumpAndSettle();
      expect(repository.captures, [
        [channel, source, 'First line'],
      ]);
      expect(
        find.text('Saved in Triage. Open Work to view the issue.'),
        findsOneWidget,
      );
    },
  );
}
