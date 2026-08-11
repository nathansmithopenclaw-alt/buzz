export type WorkflowApprovalRequest = {
  approverPubkey: string | null;
  expiresAt: Date | null;
  token: string | null;
  tokenHash: string | null;
};

function tagValue(tags: readonly string[][] | undefined, name: string) {
  return tags?.find((tag) => tag[0] === name)?.[1]?.trim() || null;
}

function isHex64(value: string | null): value is string {
  return Boolean(value && /^[0-9a-f]{64}$/i.test(value));
}

function isUuid(value: string | null): value is string {
  return Boolean(
    value &&
      /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(
        value,
      ),
  );
}

export function parseWorkflowApprovalRequest(
  tags: readonly string[][] | undefined,
): WorkflowApprovalRequest {
  const approverPubkey = tagValue(tags, "p");
  const token = tagValue(tags, "token");
  const tokenHash = tagValue(tags, "d");
  const expiration = tagValue(tags, "expiration");
  const expirationSeconds = expiration ? Number(expiration) : Number.NaN;
  const parsedExpiration =
    Number.isSafeInteger(expirationSeconds) && expirationSeconds > 0
      ? new Date(expirationSeconds * 1_000)
      : null;

  return {
    approverPubkey: isHex64(approverPubkey) ? approverPubkey : null,
    expiresAt:
      parsedExpiration && Number.isFinite(parsedExpiration.getTime())
        ? parsedExpiration
        : null,
    token: isUuid(token) ? token : null,
    tokenHash: isHex64(tokenHash) ? tokenHash : null,
  };
}
