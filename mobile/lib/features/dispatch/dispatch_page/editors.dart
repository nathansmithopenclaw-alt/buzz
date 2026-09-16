part of '../dispatch_page.dart';

class _TriageEditor extends HookConsumerWidget {
  const _TriageEditor({
    required this.repository,
    required this.workspace,
    required this.issue,
    required this.role,
  });
  final DispatchRepository repository;
  final String workspace, role;
  final Map<String, dynamic> issue;
  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final status = useState(issue['status'] as String),
        priority = useState(issue['priority'] as String?);
    final owner = useTextEditingController(
      text: issue['assignee'] as String? ?? '',
    );
    final reason = useTextEditingController();
    useListenable(owner);
    final busy = useState(false), error = useState<Object?>(null);
    final current = ref.watch(dispatchRepositoryProvider);
    final terminal = ['done', 'cancelled'].contains(issue['status']);
    final statuses = [
      'triage',
      'backlog',
      'todo',
      'in_progress',
      'in_review',
      if (role == 'maintainer') ...['done', 'cancelled'],
    ];
    if (!statuses.contains(status.value)) statuses.add(status.value);
    Future<void> save() async {
      FocusScope.of(context).unfocus();
      busy.value = true;
      error.value = null;
      try {
        if (!identical(current, repository)) {
          throw StateError(
            'The community or identity changed. Reopen this item.',
          );
        }
        final changedOwner = owner.text.trim() != (issue['assignee'] ?? '');
        await repository.edit(workspace, issue['issueId'] as String, {
          'operation': 'prepare_issue',
          'issue_id': issue['issueId'],
          'expected_tip': issue['tipEvent'],
          'set': {
            'status': status.value,
            if (priority.value != null) 'priority': priority.value,
            if (changedOwner && owner.text.trim().isNotEmpty)
              'assignee': owner.text.trim(),
          },
          'clear_assignee': changedOwner && owner.text.trim().isEmpty,
          'clear_priority': priority.value == null && issue['priority'] != null,
          'reason': reason.text.trim().isEmpty
              ? 'Updated from mobile triage'
              : reason.text.trim(),
        });
        if (context.mounted) Navigator.of(context).pop(true);
      } catch (e) {
        if (context.mounted) error.value = e;
      } finally {
        if (context.mounted) busy.value = false;
      }
    }

    return _DispatchEditorScaffold(
      title: 'Triage item',
      busy: busy.value,
      error: error.value,
      saveLabel: 'Save changes',
      savingLabel: 'Saving to Buzz…',
      onSave: save,
      children: [
        Text(issue['title'] as String, style: context.textTheme.titleLarge),
        const SizedBox(height: Grid.sm),
        DropdownButtonFormField<String>(
          initialValue: status.value,
          decoration: const InputDecoration(
            labelText: 'Status',
            border: OutlineInputBorder(),
          ),
          items: statuses
              .map(
                (item) =>
                    DropdownMenuItem(value: item, child: Text(_status(item))),
              )
              .toList(),
          onChanged: busy.value || (terminal && role != 'maintainer')
              ? null
              : (value) => status.value = value!,
        ),
        const SizedBox(height: Grid.xs),
        DropdownButtonFormField<String>(
          initialValue: priority.value ?? 'none',
          decoration: const InputDecoration(
            labelText: 'Priority',
            border: OutlineInputBorder(),
          ),
          items: const [
            DropdownMenuItem(value: 'none', child: Text('No priority')),
            DropdownMenuItem(value: 'P0', child: Text('P0 · Urgent')),
            DropdownMenuItem(value: 'P1', child: Text('P1 · High')),
            DropdownMenuItem(value: 'P2', child: Text('P2 · Normal')),
            DropdownMenuItem(value: 'P3', child: Text('P3 · Low')),
          ],
          onChanged: busy.value
              ? null
              : (value) => priority.value = value == 'none' ? null : value,
        ),
        const SizedBox(height: Grid.xs),
        Text(
          'Owner: ${_person(ref, owner.text.isEmpty ? null : owner.text, repository.pubkey)}',
        ),
        Wrap(
          spacing: Grid.xxs,
          children: [
            TextButton(
              onPressed: busy.value
                  ? null
                  : () => owner.text = repository.pubkey,
              child: const Text('Assign to me'),
            ),
            TextButton(
              onPressed: busy.value ? null : () => owner.clear(),
              child: const Text('Leave unassigned'),
            ),
          ],
        ),
        const Text(
          'Ownership identifies the person responsible. It does not start an agent.',
        ),
        const SizedBox(height: Grid.xs),
        TextField(
          controller: reason,
          maxLength: 2000,
          maxLines: 3,
          decoration: const InputDecoration(
            labelText: 'Reason for this change',
            hintText: 'Add context for your teammates',
            border: OutlineInputBorder(),
          ),
        ),
      ],
    );
  }
}

class _DecisionEditor extends HookConsumerWidget {
  const _DecisionEditor({
    required this.repository,
    required this.workspace,
    required this.issueId,
    required this.record,
  });
  final DispatchRepository repository;
  final String workspace, issueId;
  final Map<String, dynamic> record;
  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final payload = record['record']['payload'] as Map<String, dynamic>;
    final selected = useState<String?>(null),
        busy = useState(false),
        error = useState<Object?>(null);
    final rationale = useTextEditingController();
    final active = ref.watch(dispatchRepositoryProvider);
    Future<void> choose() async {
      FocusScope.of(context).unfocus();
      if (selected.value == null || rationale.text.trim().isEmpty) {
        error.value = 'Choose an option and explain your decision.';
        return;
      }
      busy.value = true;
      error.value = null;
      try {
        if (!identical(active, repository)) {
          throw StateError(
            'The community or identity changed. Reopen this decision.',
          );
        }
        await repository.edit(workspace, issueId, {
          'operation': 'prepare_decision',
          'record_id': record['record']['record_id'],
          'expected_event': record['event_id'],
          'option': selected.value,
          'rationale': rationale.text.trim(),
        }, decisionPayload: payload);
        if (context.mounted) Navigator.of(context).pop(true);
      } catch (e) {
        if (context.mounted) error.value = e;
      } finally {
        if (context.mounted) busy.value = false;
      }
    }

    return _DispatchEditorScaffold(
      title: 'Your decision',
      busy: busy.value,
      error: error.value,
      saveLabel: 'Record decision',
      savingLabel: 'Recording your decision…',
      onSave: choose,
      children: [
        Text(
          payload['question'] as String,
          style: context.textTheme.headlineSmall,
        ),
        const SizedBox(height: Grid.xs),
        if ((payload['context'] as String).isNotEmpty)
          Text(payload['context'] as String),
        const SizedBox(height: Grid.xs),
        RadioGroup<String>(
          groupValue: selected.value,
          onChanged: (value) {
            if (!busy.value) selected.value = value;
          },
          child: Column(
            children: [
              for (final option in _objects(payload['options']))
                Card(
                  child: RadioListTile<String>(
                    value: option['id'] as String,
                    title: Text(option['label'] as String),
                    subtitle: Text(option['consequence'] as String),
                    enabled: !busy.value,
                  ),
                ),
            ],
          ),
        ),
        if (_objects(payload['evidence']).isNotEmpty) ...[
          Text('Evidence to review', style: context.textTheme.titleMedium),
          for (final reference in _objects(payload['evidence']))
            Padding(
              padding: const EdgeInsets.symmetric(vertical: Grid.xxs),
              child: SelectableText(reference['ref'] as String),
            ),
        ],
        const SizedBox(height: Grid.xs),
        TextField(
          controller: rationale,
          maxLines: 4,
          maxLength: 4000,
          decoration: const InputDecoration(
            labelText: 'Why this option?',
            border: OutlineInputBorder(),
          ),
        ),
        const SizedBox(height: Grid.xxs),
        const Text(
          'This records your choice. It does not automatically close the work or mark delivery as verified.',
        ),
      ],
    );
  }
}
