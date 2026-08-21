import { execFileSync } from "node:child_process";
import { promises as fs } from "node:fs";
import path from "node:path";

function git(args, cwd, options = {}) {
  // Git hooks export repository-local GIT_* variables. Child commands that
  // intentionally target `cwd` must not be redirected back to the hook's repo.
  const env = Object.fromEntries(
    Object.entries(process.env).filter(([key]) => !key.startsWith("GIT_")),
  );
  return execFileSync("git", args, {
    cwd,
    encoding: "utf8",
    maxBuffer: 10 * 1024 * 1024,
    env,
    ...options,
  });
}

function toPosixPath(relativePath) {
  return relativePath.split(path.sep).join("/");
}

export function countLines(content) {
  if (content.length === 0) {
    return 0;
  }
  return content.split(/\r?\n/).length;
}

export function allowedLineCount(baseLines, maxLines) {
  return baseLines == null || baseLines <= maxLines ? maxLines : baseLines;
}

export function evaluateFileSize({ baseLines, candidateLines, maxLines }) {
  const limit = allowedLineCount(baseLines, maxLines);
  return { limit, violates: candidateLines > limit };
}

function findRule(rules, relativePath) {
  return rules.find((rule) => relativePath.startsWith(`${rule.root}/`));
}

export function resolveBaseRef(repoRoot, env = process.env) {
  if (env.CHECK_FILE_SIZES_BASE) {
    return env.CHECK_FILE_SIZES_BASE;
  }

  if (env.GITHUB_ACTIONS === "true") {
    return "HEAD^1";
  }

  try {
    const mergeBase = git(
      ["merge-base", "origin/main", "HEAD"],
      repoRoot,
    ).trim();
    const head = git(["rev-parse", "HEAD"], repoRoot).trim();
    return mergeBase === head ? "HEAD" : mergeBase;
  } catch (error) {
    throw new Error(
      "Could not resolve the file-size base from origin/main. Fetch origin/main or set CHECK_FILE_SIZES_BASE to an explicit commit.",
      { cause: error },
    );
  }
}

/**
 * Resolve every parent whose already-accepted file sizes a merge candidate may
 * inherit. A normal feature branch keeps the historical single merge-base
 * behavior. An in-progress or committed merge compares against both parents,
 * so unchanged files from either accepted line are grandfathered without
 * allowing merge-only growth beyond the larger parent.
 */
export function resolveBaseRefs(repoRoot, env = process.env) {
  if (env.CHECK_FILE_SIZES_BASE) {
    return [env.CHECK_FILE_SIZES_BASE];
  }

  try {
    const mergeHead = git(
      ["rev-parse", "--quiet", "--verify", "MERGE_HEAD"],
      repoRoot,
      { stdio: ["ignore", "pipe", "ignore"] },
    ).trim();
    if (mergeHead) {
      return [git(["rev-parse", "HEAD"], repoRoot).trim(), mergeHead];
    }
  } catch {
    // No merge is in progress.
  }

  const headAndParents = git(
    ["rev-list", "--parents", "-n", "1", "HEAD"],
    repoRoot,
  )
    .trim()
    .split(/\s+/);
  if (headAndParents.length > 2) {
    return headAndParents.slice(1);
  }

  return [resolveBaseRef(repoRoot, env)];
}

export function parseChangedFiles(output) {
  const fields = output.split("\0");
  const changes = [];

  for (let index = 0; index < fields.length - 1; ) {
    const status = fields[index++];
    if (status.startsWith("R") || status.startsWith("C")) {
      changes.push({
        status: status[0],
        oldPath: fields[index++],
        path: fields[index++],
      });
    } else {
      changes.push({ status: status[0], path: fields[index++] });
    }
  }

  return changes;
}

function changedProjectFiles({ repoRoot, projectRelative, baseRef }) {
  const output = git(
    ["diff", "--name-status", "-z", "-M", baseRef, "--", projectRelative],
    repoRoot,
  );
  const changes = parseChangedFiles(output);
  const trackedPaths = new Set(changes.map((change) => change.path));
  const untracked = git(
    ["ls-files", "--others", "--exclude-standard", "-z", "--", projectRelative],
    repoRoot,
  )
    .split("\0")
    .filter(Boolean);

  for (const filePath of untracked) {
    if (!trackedPaths.has(filePath)) {
      changes.push({ status: "A", path: filePath });
    }
  }
  return changes;
}

function readBaseFile(repoRoot, baseRef, filePath) {
  try {
    return git(["show", `${baseRef}:${filePath}`], repoRoot, {
      encoding: null,
      stdio: ["ignore", "pipe", "ignore"],
    }).toString("utf8");
  } catch {
    return null;
  }
}

export async function runFileSizeCheck({ projectRoot, rules, label }) {
  // Every governed project is a direct child of the repository root. Derive
  // these paths without Git so hook-provided repository environment variables
  // cannot collapse the project pathspec to an empty string.
  const repoRoot = path.dirname(projectRoot);
  const projectRelative = toPosixPath(path.basename(projectRoot));
  const baseRefs = resolveBaseRefs(repoRoot);

  // Fail clearly instead of silently turning a missing/shallow base into a pass.
  for (const baseRef of baseRefs) {
    git(["cat-file", "-e", `${baseRef}^{commit}`], repoRoot);
  }

  const violations = [];
  const changesByPath = new Map();
  for (const baseRef of baseRefs) {
    for (const change of changedProjectFiles({
      repoRoot,
      projectRelative,
      baseRef,
    })) {
      const existing = changesByPath.get(change.path) ?? {
        ...change,
        basePaths: new Map(),
      };
      existing.basePaths.set(baseRef, change.oldPath ?? change.path);
      changesByPath.set(change.path, existing);
    }
  }

  for (const change of changesByPath.values()) {
    if (change.status === "D") continue;

    const relativePath = toPosixPath(
      path.relative(projectRelative, change.path),
    );
    const rule = findRule(rules, relativePath);
    if (!rule || !rule.extensions.has(path.extname(relativePath))) continue;

    const candidatePath = path.join(repoRoot, change.path);
    const candidateLines = countLines(await fs.readFile(candidatePath, "utf8"));
    const baseLineCounts = baseRefs.flatMap((baseRef) => {
      const basePath = change.basePaths.get(baseRef) ?? change.path;
      const baseContent = readBaseFile(repoRoot, baseRef, basePath);
      return baseContent == null ? [] : [countLines(baseContent)];
    });
    const baseLines =
      baseLineCounts.length === 0 ? null : Math.max(...baseLineCounts);
    const result = evaluateFileSize({
      baseLines,
      candidateLines,
      maxLines: rule.maxLines,
    });

    if (result.violates) {
      violations.push({
        relativePath,
        baseLines,
        candidateLines,
        limit: result.limit,
      });
    }
  }

  if (violations.length === 0) return;

  console.error(
    `${label} file size ratchet failed (base ${baseRefs.join(", ")}):`,
  );
  for (const violation of violations) {
    const before = violation.baseLines == null ? "new" : violation.baseLines;
    const delta =
      violation.baseLines == null
        ? ""
        : ` (${violation.candidateLines - violation.baseLines >= 0 ? "+" : ""}${violation.candidateLines - violation.baseLines})`;
    console.error(
      `- ${violation.relativePath}: ${before} -> ${violation.candidateLines}${delta} lines (allowed ${violation.limit})`,
    );
  }
  console.error(
    "Keep new files at or below the limit; files already over it may not grow.",
  );
  process.exitCode = 1;
}
