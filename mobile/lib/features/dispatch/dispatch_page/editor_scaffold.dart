part of '../dispatch_page.dart';

/// Keep the save action and its result visible, including above the keyboard.
class _DispatchEditorScaffold extends StatelessWidget {
  const _DispatchEditorScaffold({
    required this.title,
    required this.busy,
    required this.error,
    required this.saveLabel,
    required this.savingLabel,
    required this.onSave,
    required this.children,
  });

  final String title, saveLabel, savingLabel;
  final bool busy;
  final Object? error;
  final VoidCallback onSave;
  final List<Widget> children;

  @override
  Widget build(BuildContext context) => PopScope(
    canPop: !busy,
    child: FrostedScaffold(
      appBar: FrostedAppBar(title: Text(title)),
      body: Column(
        children: [
          Expanded(
            child: ListView(
              keyboardDismissBehavior: ScrollViewKeyboardDismissBehavior.onDrag,
              padding: EdgeInsets.fromLTRB(
                Grid.gutter,
                frostedAppBarHeight(context) + Grid.xs,
                Grid.gutter,
                Grid.xs,
              ),
              children: children,
            ),
          ),
          SafeArea(
            top: false,
            child: Padding(
              padding: const EdgeInsets.fromLTRB(
                Grid.gutter,
                Grid.xxs,
                Grid.gutter,
                Grid.xs,
              ),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  if (error != null) ...[
                    Text(
                      'Save not confirmed',
                      style: context.textTheme.titleSmall?.copyWith(
                        color: context.colors.error,
                      ),
                    ),
                    _error(error!),
                    TextButton(
                      onPressed: () => Navigator.of(context).pop(false),
                      child: const Text('Back to item'),
                    ),
                  ],
                  FilledButton.icon(
                    onPressed: busy ? null : onSave,
                    icon: busy
                        ? const SizedBox.square(
                            dimension: 18,
                            child: CircularProgressIndicator(strokeWidth: 2),
                          )
                        : const Icon(Icons.check),
                    label: Text(busy ? savingLabel : saveLabel),
                  ),
                ],
              ),
            ),
          ),
        ],
      ),
    ),
  );
}
