import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import '../theme/theme.dart';
import '../widgets/modal_presentation.dart';
import 'dispatch_repository.dart';

/// Review a source-linked issue before publishing it into the same channel.
void showDispatchCaptureSheet(
  BuildContext context,
  String channel,
  String source,
  String content,
) {
  showBuzzModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    showDragHandle: true,
    builder: (_) => DispatchCaptureSheet(
      channel: channel,
      source: source,
      content: content,
    ),
  );
}

class DispatchCaptureSheet extends HookConsumerWidget {
  const DispatchCaptureSheet({
    super.key,
    required this.channel,
    required this.source,
    required this.content,
  });
  final String channel, source, content;
  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final current = ref.watch(dispatchRepositoryProvider);
    final repository = useRef(current).value;
    final title = useTextEditingController(
      text: content.split('\n').first.characters.take(256).toString(),
    );
    final busy = useState(false), saved = useState(false);
    final error = useState<Object?>(null);
    useListenable(title);
    Future<void> save() async {
      busy.value = true;
      error.value = null;
      try {
        if (!identical(current, repository)) {
          throw StateError(
            'The community or identity changed. Reopen this message.',
          );
        }
        await repository.capture(channel, source, title.text);
        if (context.mounted) saved.value = true;
      } catch (cause) {
        if (context.mounted) error.value = cause;
      } finally {
        if (context.mounted) busy.value = false;
      }
    }

    return SafeArea(
      child: Padding(
        padding: EdgeInsets.fromLTRB(
          Grid.gutter,
          Grid.sm,
          Grid.gutter,
          MediaQuery.viewInsetsOf(context).bottom + Grid.gutter,
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text('Track in Dispatch', style: context.textTheme.titleLarge),
            const SizedBox(height: Grid.sm),
            if (saved.value) ...[
              const Text('Saved in Triage. Open Work to view the issue.'),
              TextButton(
                onPressed: () => Navigator.of(context).pop(),
                child: const Text('Done'),
              ),
            ] else ...[
              TextField(
                controller: title,
                enabled: !busy.value,
                maxLength: 256,
                decoration: const InputDecoration(
                  labelText: 'Outcome or next step',
                ),
              ),
              const Text(
                'Creates an issue in this channel with a link back to the message.',
              ),
              if (error.value != null)
                Text(
                  error.value.toString(),
                  style: TextStyle(color: context.colors.error),
                ),
              const SizedBox(height: Grid.sm),
              FilledButton(
                onPressed: busy.value || title.text.trim().isEmpty
                    ? null
                    : save,
                child: Text(busy.value ? 'Saving…' : 'Create linked issue'),
              ),
            ],
          ],
        ),
      ),
    );
  }
}
