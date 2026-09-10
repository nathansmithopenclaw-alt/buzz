import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/profile/user_cache_provider.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import 'dispatch_repository.dart';

part 'dispatch_page/workspace.dart';
part 'dispatch_page/detail.dart';
part 'dispatch_page/editors.dart';
part 'dispatch_page/saved_drafts.dart';

/// Focused mobile work inbox: triage, ownership and explicit decisions.
class DispatchPage extends HookConsumerWidget {
  const DispatchPage({super.key});
  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final repository = ref.watch(dispatchRepositoryProvider);
    return _WorkspacePage(
      key: ValueKey('${repository.relay}:${repository.pubkey}'),
      repository: repository,
    );
  }
}

String _status(String value) => switch (value) {
  'triage' => 'Triage',
  'backlog' => 'Backlog',
  'todo' => 'To do',
  'in_progress' => 'In progress',
  'in_review' => 'In review',
  'done' => 'Done',
  'cancelled' => 'Cancelled',
  _ => value,
};
String _person(WidgetRef ref, String? key, String viewer) {
  if (key == null) return 'Unassigned';
  if (key == viewer) return 'You';
  ref.watch(userCacheProvider);
  return ref.read(userCacheProvider.notifier).get(key)?.label ??
      'Member ${key.substring(0, 8)}';
}

List<Map<String, dynamic>> _objects(Object? value) =>
    (value as List? ?? []).cast<Map<String, dynamic>>();
Widget _error(Object error) => Semantics(
  liveRegion: true,
  child: Padding(
    padding: const EdgeInsets.all(Grid.xs),
    child: Text(
      error.toString().replaceFirst(
        RegExp(r'^(Bad state: |Exception: |FormatException: )'),
        '',
      ),
    ),
  ),
);

class _Recovery extends HookWidget {
  const _Recovery({
    required this.repository,
    required this.workspace,
    required this.onChanged,
    required this.generation,
  });
  final DispatchRepository repository;
  final String workspace;
  final VoidCallback onChanged;
  final int generation;
  @override
  Widget build(BuildContext context) {
    final check = useFuture(
      useMemoized(() => repository.pending(workspace), [
        repository,
        workspace,
        generation,
      ]),
    );
    final busy = useState(false), error = useState<Object?>(null);
    if (check.hasError) return _error(check.error!);
    if (check.data == null) return const SizedBox.shrink();
    Future<void> run(Future<void> Function() operation) async {
      busy.value = true;
      error.value = null;
      try {
        await operation();
        if (context.mounted) onChanged();
      } catch (e) {
        if (context.mounted) error.value = e;
      } finally {
        if (context.mounted) busy.value = false;
      }
    }

    return Card(
      child: Padding(
        padding: const EdgeInsets.all(Grid.xs),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const Text(
              'An edit is saved on this device',
              style: TextStyle(fontWeight: FontWeight.w600),
            ),
            const SizedBox(height: Grid.xxs),
            const Text(
              'Check whether it was accepted before starting another edit. Retrying uses the same saved edit.',
            ),
            if (error.value != null) _error(error.value!),
            Wrap(
              spacing: Grid.xxs,
              children: [
                FilledButton(
                  onPressed: busy.value
                      ? null
                      : () => run(() => repository.resume(workspace)),
                  child: Text(busy.value ? 'Checking…' : 'Check and retry'),
                ),
                TextButton(
                  onPressed: busy.value
                      ? null
                      : () =>
                            run(() => repository.keepPendingAsDraft(workspace)),
                  child: const Text('Keep as a draft'),
                ),
                TextButton(
                  onPressed: busy.value
                      ? null
                      : () => Clipboard.setData(
                          ClipboardData(text: _draftText(check.data!.toJson())),
                        ),
                  child: const Text('Copy saved edit'),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}
