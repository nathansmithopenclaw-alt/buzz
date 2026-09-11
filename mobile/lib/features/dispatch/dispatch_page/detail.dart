part of '../dispatch_page.dart';

class _IssuePage extends HookConsumerWidget {
  const _IssuePage({
    required this.repository,
    required this.workspace,
    required this.issueId,
  });
  final DispatchRepository repository;
  final String workspace;
  final String issueId;
  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final active = ref.watch(dispatchRepositoryProvider);
    final refresh = useState(0);
    final snapshot = useFuture(
      useMemoized(() => repository.snapshot(workspace, issueId: issueId), [
        repository,
        workspace,
        issueId,
        refresh.value,
      ]),
    );
    final value = snapshot.data;
    final issue = _objects(value?['issues']).firstOrNull;
    Future<void> edit() async {
      if (issue == null || value == null) return;
      final saved = await Navigator.of(context).push<bool>(
        MaterialPageRoute(
          builder: (_) => _TriageEditor(
            repository: repository,
            workspace: workspace,
            issue: issue,
            role: value['viewerRole'] as String,
          ),
        ),
      );
      if (context.mounted) {
        refresh.value++;
        if (saved == true) {
          ScaffoldMessenger.of(context).showSnackBar(
            const SnackBar(content: Text('Changes saved to Buzz')),
          );
        }
      }
    }

    return FrostedScaffold(
      appBar: FrostedAppBar(
        title: Text(issue?['identifier'] as String? ?? 'Work item'),
        actions: [
          IconButton(
            tooltip: 'Refresh item',
            onPressed: () => refresh.value++,
            icon: const Icon(Icons.refresh),
          ),
        ],
      ),
      body: !identical(active, repository)
          ? _error(
              'This view belongs to another community or identity. Reopen Dispatch from Work.',
            )
          : ListView(
              padding: EdgeInsets.fromLTRB(
                Grid.gutter,
                frostedAppBarHeight(context) + Grid.xs,
                Grid.gutter,
                Grid.lg,
              ),
              children: [
                if (snapshot.connectionState == ConnectionState.waiting)
                  const LinearProgressIndicator(),
                if (snapshot.hasError) _error(snapshot.error!),
                _Recovery(
                  repository: repository,
                  workspace: workspace,
                  generation: refresh.value,
                  onChanged: () => refresh.value++,
                ),
                if (issue != null) ...[
                  Text(
                    issue['title'] as String,
                    style: context.textTheme.headlineSmall?.copyWith(
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                  const SizedBox(height: Grid.xs),
                  Wrap(
                    spacing: Grid.xxs,
                    runSpacing: Grid.half,
                    children: [
                      Chip(
                        label: Text(_status(issue['status'] as String)),
                        labelStyle: TextStyle(color: context.colors.onSurface),
                      ),
                      if (issue['priority'] != null)
                        Chip(
                          label: Text(issue['priority'] as String),
                          labelStyle: TextStyle(
                            color: context.colors.onSurface,
                          ),
                        ),
                    ],
                  ),
                  Text(
                    'Owner: ${_person(ref, issue['assignee'] as String?, repository.pubkey)}',
                  ),
                  const SizedBox(height: Grid.xs),
                  FilledButton.tonal(
                    onPressed: snapshot.hasError ? null : edit,
                    child: const Text('Triage this item'),
                  ),
                  const SizedBox(height: Grid.sm),
                  if ((issue['body'] as String).isNotEmpty)
                    SelectableText(
                      issue['body'] as String,
                      style: context.textTheme.bodyLarge,
                    ),
                  if ((issue['labels'] as List).isNotEmpty)
                    Padding(
                      padding: const EdgeInsets.only(top: Grid.xs),
                      child: Wrap(
                        spacing: Grid.xxs,
                        children: [
                          for (final label in issue['labels'] as List)
                            Chip(
                              label: Text(label as String),
                              labelStyle: TextStyle(
                                color: context.colors.onSurface,
                              ),
                            ),
                        ],
                      ),
                    ),
                  const SizedBox(height: Grid.sm),
                  for (final record in _objects(value?['decisions']))
                    _DecisionCard(
                      repository: repository,
                      workspace: workspace,
                      issueId: issueId,
                      record: record,
                      onChanged: () => refresh.value++,
                    ),
                  if (_objects(issue['externalRefs']).isNotEmpty) ...[
                    const Divider(),
                    Text(
                      'Links and evidence',
                      style: context.textTheme.titleMedium,
                    ),
                    for (final link in _objects(issue['externalRefs']).where(
                      (link) => [
                        'url',
                        'source',
                        'document',
                        'attachment',
                      ].contains(link['type']),
                    ))
                      ListTile(
                        contentPadding: EdgeInsets.zero,
                        title: Text(link['type'] as String),
                        subtitle: SelectableText(link['ref'] as String),
                        trailing: IconButton(
                          tooltip: 'Copy link',
                          icon: const Icon(Icons.copy),
                          onPressed: () => Clipboard.setData(
                            ClipboardData(text: link['ref'] as String),
                          ),
                        ),
                      ),
                  ],
                ],
              ],
            ),
    );
  }
}

class _DecisionCard extends HookConsumerWidget {
  const _DecisionCard({
    required this.repository,
    required this.workspace,
    required this.issueId,
    required this.record,
    required this.onChanged,
  });
  final DispatchRepository repository;
  final String workspace, issueId;
  final Map<String, dynamic> record;
  final VoidCallback onChanged;
  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final payload = record['record']['payload'] as Map<String, dynamic>;
    final selected = payload['selected_option'] as String?;
    final options = _objects(payload['options']);
    final archived = record['record']['archived'] == true;
    Future<void> decide() async {
      final saved = await Navigator.of(context).push<bool>(
        MaterialPageRoute(
          builder: (_) => _DecisionEditor(
            repository: repository,
            workspace: workspace,
            issueId: issueId,
            record: record,
          ),
        ),
      );
      if (context.mounted) {
        onChanged();
        if (saved == true) {
          ScaffoldMessenger.of(context).showSnackBar(
            const SnackBar(content: Text('Decision recorded in Buzz')),
          );
        }
      }
    }

    return Card(
      child: Padding(
        padding: const EdgeInsets.all(Grid.xs),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              payload['question'] as String,
              style: context.textTheme.titleMedium,
            ),
            const SizedBox(height: Grid.xxs),
            Text(
              archived
                  ? 'Archived decision'
                  : selected == null
                  ? 'Waiting for ${_person(ref, payload['decider'] as String, repository.pubkey)}'
                  : 'Chosen: ${options.where((option) => option['id'] == selected).firstOrNull?['label'] ?? selected}',
            ),
            if ((payload['context'] as String).isNotEmpty)
              Padding(
                padding: const EdgeInsets.only(top: Grid.xxs),
                child: Text(payload['context'] as String),
              ),
            if (payload['rationale'] != null)
              Padding(
                padding: const EdgeInsets.only(top: Grid.xxs),
                child: Text(payload['rationale'] as String),
              ),
            if (!archived &&
                selected == null &&
                payload['decider'] == repository.pubkey)
              Padding(
                padding: const EdgeInsets.only(top: Grid.xs),
                child: FilledButton(
                  onPressed: decide,
                  child: const Text('Review options and decide'),
                ),
              ),
          ],
        ),
      ),
    );
  }
}
