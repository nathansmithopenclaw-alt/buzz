part of '../dispatch_page.dart';

class _WorkspacePage extends HookConsumerWidget {
  const _WorkspacePage({super.key, required this.repository});
  final DispatchRepository repository;
  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final refresh = useState(0), selected = useState<String?>(null);
    final workspaces = useFuture(
      useMemoized(repository.workspaces, [repository, refresh.value]),
    );
    final choices = workspaces.data ?? [];
    final workspace = choices.any((choice) => choice.id == selected.value)
        ? selected.value
        : (choices.isNotEmpty ? choices.first.id : null);
    return FrostedScaffold(
      appBar: FrostedAppBar(
        title: const Text('Dispatch'),
        automaticallyImplyLeading: false,
        actions: [
          IconButton(
            tooltip: 'Refresh work',
            onPressed: () => refresh.value++,
            icon: const Icon(Icons.refresh),
          ),
        ],
      ),
      body: Padding(
        padding: EdgeInsets.only(top: frostedAppBarHeight(context)),
        child: Column(
          children: [
            if (workspaces.hasError) _error(workspaces.error!),
            if (workspaces.connectionState == ConnectionState.waiting)
              const LinearProgressIndicator(),
            if (choices.isNotEmpty)
              Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: Grid.gutter,
                  vertical: Grid.xxs,
                ),
                child: DropdownButtonFormField<String>(
                  key: ValueKey(workspace),
                  initialValue: workspace,
                  isExpanded: true,
                  decoration: const InputDecoration(
                    labelText: 'Workspace',
                    border: OutlineInputBorder(),
                  ),
                  items: choices
                      .map(
                        (item) => DropdownMenuItem(
                          value: item.id,
                          child: Text(
                            item.name,
                            overflow: TextOverflow.ellipsis,
                          ),
                        ),
                      )
                      .toList(),
                  onChanged: (value) => selected.value = value,
                ),
              ),
            if (workspaces.connectionState == ConnectionState.done &&
                !workspaces.hasError &&
                workspace == null)
              const Padding(
                padding: EdgeInsets.all(Grid.gutter),
                child: Text(
                  'No Dispatch workspaces were found in this community. Create your first workspace on Desktop.',
                ),
              ),
            if (workspace != null)
              Expanded(
                child: _WorkQueue(
                  key: ValueKey(workspace),
                  repository: repository,
                  workspace: workspace,
                  generation: refresh.value,
                  onRefresh: () => refresh.value++,
                ),
              ),
          ],
        ),
      ),
    );
  }
}

class _WorkQueue extends HookConsumerWidget {
  const _WorkQueue({
    super.key,
    required this.repository,
    required this.workspace,
    required this.generation,
    required this.onRefresh,
  });
  final DispatchRepository repository;
  final String workspace;
  final int generation;
  final VoidCallback onRefresh;
  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final snapshot = useFuture(
      useMemoized(() => repository.allOpen(workspace), [
        repository,
        workspace,
        generation,
      ]),
    );
    final filter = useState('mine'), search = useState('');
    final value = snapshot.data;
    final issues = _objects(value?['issues']);
    final decisions = _objects(value?['decisions']);
    final mine = issues
        .where((issue) => issue['assignee'] == repository.pubkey)
        .length;
    final waiting = decisions
        .where(
          (item) => (item['record']['payload']['decider']) == repository.pubkey,
        )
        .length;
    Future<void> open(String issueId) async {
      await Navigator.of(context).push<void>(
        MaterialPageRoute(
          builder: (_) => _IssuePage(
            repository: repository,
            workspace: workspace,
            issueId: issueId,
          ),
        ),
      );
      if (context.mounted) onRefresh();
    }

    return RefreshIndicator(
      onRefresh: () async => onRefresh(),
      child: ListView(
        padding: const EdgeInsets.fromLTRB(
          Grid.gutter,
          Grid.xxs,
          Grid.gutter,
          120,
        ),
        children: [
          _Recovery(
            repository: repository,
            workspace: workspace,
            generation: generation,
            onChanged: onRefresh,
          ),
          _SavedDrafts(
            repository: repository,
            workspace: workspace,
            generation: generation,
            onChanged: onRefresh,
            openIssue: open,
          ),
          Text(
            'Your work',
            style: context.textTheme.headlineSmall?.copyWith(
              fontWeight: FontWeight.w600,
            ),
          ),
          const SizedBox(height: Grid.xxs),
          Text(
            value == null
                ? 'Loading your work…'
                : '$mine open ${mine == 1 ? 'item' : 'items'} owned by you · $waiting ${waiting == 1 ? 'decision' : 'decisions'} waiting',
            style: context.textTheme.bodyMedium,
          ),
          const SizedBox(height: Grid.xs),
          Wrap(
            spacing: Grid.xxs,
            runSpacing: Grid.half,
            children: [
              for (final option in [
                ('mine', 'My work'),
                ('decisions', 'Decisions'),
                ('all', 'All open'),
              ])
                ChoiceChip(
                  label: Text(option.$2),
                  selectedColor: context.colors.primary,
                  checkmarkColor: context.colors.onPrimary,
                  labelStyle: TextStyle(
                    color: filter.value == option.$1
                        ? context.colors.onPrimary
                        : context.colors.onSurface,
                  ),
                  selected: filter.value == option.$1,
                  onSelected: (_) => filter.value = option.$1,
                ),
            ],
          ),
          const SizedBox(height: Grid.xxs),
          TextField(
            onChanged: (value) => search.value = value,
            decoration: const InputDecoration(
              prefixIcon: Icon(Icons.search),
              hintText: 'Find work by title or label',
              border: OutlineInputBorder(),
            ),
          ),
          if (snapshot.connectionState == ConnectionState.waiting)
            const Padding(
              padding: EdgeInsets.all(Grid.xs),
              child: LinearProgressIndicator(),
            ),
          if (snapshot.hasError) _error(snapshot.error!),
          if (value != null && filter.value == 'decisions') ...[
            for (final decision in decisions.where(
              (item) =>
                  item['record']['payload']['decider'] == repository.pubkey &&
                  item['record']['payload']['question']
                      .toString()
                      .toLowerCase()
                      .contains(search.value.toLowerCase()),
            ))
              Card(
                child: ListTile(
                  leading: const Icon(Icons.help_outline),
                  title: Text(
                    decision['record']['payload']['question'] as String,
                  ),
                  subtitle: const Text('Your decision is needed'),
                  trailing: const Icon(Icons.chevron_right),
                  onTap: () =>
                      open(decision['record']['payload']['issue_id'] as String),
                ),
              ),
            if (waiting == 0)
              const Padding(
                padding: EdgeInsets.all(Grid.sm),
                child: Text('No decisions are waiting for you.'),
              ),
          ],
          if (value != null && filter.value != 'decisions') ...[
            for (final issue in issues.where(
              (item) =>
                  (filter.value == 'all' ||
                      item['assignee'] == repository.pubkey) &&
                  '${item['title']} ${item['identifier']} ${(item['labels'] as List).join(' ')}'
                      .toLowerCase()
                      .contains(search.value.toLowerCase()),
            ))
              Card(
                child: ListTile(
                  contentPadding: const EdgeInsets.symmetric(
                    horizontal: Grid.xs,
                    vertical: Grid.xxs,
                  ),
                  title: Text(
                    issue['title'] as String,
                    style: const TextStyle(fontWeight: FontWeight.w600),
                  ),
                  subtitle: Padding(
                    padding: const EdgeInsets.only(top: Grid.xxs),
                    child: Text(
                      '${issue['identifier']} · ${_status(issue['status'] as String)}\n${_person(ref, issue['assignee'] as String?, repository.pubkey)}${issue['priority'] == null ? '' : ' · ${issue['priority']}'}',
                    ),
                  ),
                  isThreeLine: true,
                  trailing: const Icon(Icons.chevron_right),
                  onTap: () => open(issue['issueId'] as String),
                ),
              ),
            if ((filter.value == 'mine' ? mine : issues.length) == 0)
              const Padding(
                padding: EdgeInsets.all(Grid.sm),
                child: Text('No open work in this view.'),
              ),
          ],
          if (value != null && (value['admission']['unsealed'] as int) > 0)
            Padding(
              padding: const EdgeInsets.only(top: Grid.xs),
              child: Text(
                'Some older edits predate recorded permissions. Their interpretation follows current membership until an administrator preserves a reviewed baseline.',
                style: context.textTheme.bodySmall,
              ),
            ),
          if (value != null &&
              _objectsOrStrings(value['diagnostics']).isNotEmpty)
            const Padding(
              padding: EdgeInsets.only(top: Grid.xxs),
              child: Text(
                'Some history needs review. Desktop shows the preserved changes and conflict details.',
              ),
            ),
        ],
      ),
    );
  }
}

List<Object?> _objectsOrStrings(Object? value) =>
    (value as List? ?? []).cast<Object?>();
