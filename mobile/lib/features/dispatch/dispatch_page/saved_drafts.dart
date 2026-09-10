part of '../dispatch_page.dart';

String _draftText(Map<String, dynamic> draft) {
  final request = draft['request'] as Map<String, dynamic>;
  final lines = <String>['Saved Dispatch edit', 'Issue: ${draft['issueId']}'];
  if (request['operation'] == 'prepare_decision') {
    final baseline = request['_baseline_payload'] as Map<String, dynamic>?;
    lines.add('Decision: ${baseline?['question'] ?? request['record_id']}');
    final option = _objects(
      baseline?['options'],
    ).where((item) => item['id'] == request['option']);
    lines.add(
      'Choice: ${option.isEmpty ? request['option'] : option.first['label']}',
    );
    lines.add('Rationale: ${request['rationale']}');
  } else {
    final fields = request['set'] as Map<String, dynamic>? ?? {};
    for (final entry in fields.entries.where((entry) => entry.value != null)) {
      lines.add(
        '${entry.key == 'assignee' ? 'Owner' : entry.key}: ${entry.key == 'status' ? _status(entry.value as String) : entry.value}',
      );
    }
    if (request['clear_assignee'] == true) lines.add('Owner: Unassigned');
    if (request['clear_priority'] == true) lines.add('Priority: None');
    lines.add('Reason: ${request['reason']}');
  }
  lines.add(
    'Check the current issue before reapplying this draft. A previous send may already have been accepted.',
  );
  return lines.join('\n');
}

class _SavedDrafts extends HookWidget {
  const _SavedDrafts({
    required this.repository,
    required this.workspace,
    required this.generation,
    required this.onChanged,
    required this.openIssue,
  });
  final DispatchRepository repository;
  final String workspace;
  final int generation;
  final VoidCallback onChanged;
  final Future<void> Function(String) openIssue;
  @override
  Widget build(BuildContext context) {
    final saved = useFuture(
      useMemoized(() => repository.savedDrafts(workspace), [
        repository,
        workspace,
        generation,
      ]),
    );
    final busy = useState(false), error = useState<Object?>(null);
    if (saved.hasError) return _error(saved.error!);
    final drafts = _objects(jsonDecode(saved.data ?? '[]'));
    if (drafts.isEmpty) return const SizedBox.shrink();
    return Card(
      child: ExpansionTile(
        leading: const Icon(Icons.drafts_outlined),
        title: Text('Saved drafts (${drafts.length})'),
        subtitle: const Text(
          'Review the current issue before reapplying a draft',
        ),
        childrenPadding: const EdgeInsets.all(Grid.xs),
        children: [
          if (error.value != null) _error(error.value!),
          for (final draft in drafts)
            Padding(
              padding: const EdgeInsets.only(bottom: Grid.xs),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  SelectableText(_draftText(draft)),
                  Wrap(
                    spacing: Grid.xxs,
                    children: [
                      TextButton(
                        onPressed: busy.value
                            ? null
                            : () => openIssue(draft['issueId'] as String),
                        child: const Text('Review current issue'),
                      ),
                      TextButton(
                        onPressed: () => Clipboard.setData(
                          ClipboardData(text: _draftText(draft)),
                        ),
                        child: const Text('Copy draft'),
                      ),
                      TextButton(
                        onPressed: busy.value
                            ? null
                            : () async {
                                busy.value = true;
                                error.value = null;
                                try {
                                  await repository.removeSavedDraft(
                                    workspace,
                                    draft['event']['id'] as String,
                                  );
                                  if (context.mounted) onChanged();
                                } catch (e) {
                                  if (context.mounted) error.value = e;
                                } finally {
                                  if (context.mounted) busy.value = false;
                                }
                              },
                        child: const Text('Remove saved draft'),
                      ),
                    ],
                  ),
                ],
              ),
            ),
        ],
      ),
    );
  }
}
