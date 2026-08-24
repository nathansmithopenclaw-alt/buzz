/**
 * Buzz publish tool — owned Smith Crafts plugin for the dsh `buzz-acp` profile.
 *
 * Why this exists: dsh's bash tool scrubs credential-shaped env names
 * (`SENSITIVE_ENV_PATTERN`, /KEY|PASSWORD|SECRET|TOKEN/i) at the tool-child
 * boundary, so `buzz messages send` from the model shell always dies on the
 * missing BUZZ_PRIVATE_KEY (proven live, D1 Task 1 probe). This plugin runs
 * IN-PROCESS: it reads the credentials from the harness process environment it
 * inherited from buzz-acp's spawn and hands them to the buzz CLI via an
 * explicit env layer, which upstream explicitly supports ("a deliberately
 * supplied entry survives because explicit env layers merge after the scrub").
 *
 * CREDENTIALS DISCIPLINE (binding, Helm 2026-08-24): BUZZ_PRIVATE_KEY and
 * BUZZ_AUTH_TAG travel ONLY through the dsh process environment into the CLI
 * subprocess. They NEVER appear in tool args, the transcript, plugin config,
 * logs, or ANY file inside the nest — this plugin materializes nothing to
 * disk. There is deliberately no fallback that writes or reads a credentials
 * file: redaction of tool output is the boundary that keeps secret values out
 * of the model transcript, and a disk copy would give a misbehaving model a
 * cat-able target (observed in the D1 rig).
 *
 * DUPLICATE-PUBLISH GUARD (binding, Helm 2026-08-24): a relay returning an
 * unexpected receipt shape once made local-v4 retry-storm six identical
 * publishes. This plugin therefore remembers the last SUCCESSFUL publish per
 * agent session; an identical (channel, content, reply-to) request returns the
 * prior event id without invoking the CLI again. Scope note: dsh's tool
 * execution context exposes no turn token, so the guard keys on "most recent
 * successful publish of this session" — which is precisely the consecutive-
 * retry storm class; a genuinely new publish (different args) always passes.
 * Everything else stays fail-loud verbatim.
 *
 * Contract (Helm plan-gate 2026-08-24): args mirror the CLI surface the Buzz
 * harness prompt teaches; returns the published event id; CLI errors surface
 * verbatim to the model — fail-loud, never swallow.
 *
 * @module @smith-crafts/dsh-buzz-publish
 */

import { execFile } from 'node:child_process'
import { defineTool } from '@deepseek-ai/dsh-tools'
import z from '@deepseek-ai/schemastery'

export const name = 'buzz-publish'

export const inject = ['tools']

/** Deployment knobs. Deliberately excludes anything credential-shaped. */
export const Config = z.object({
  /** Buzz CLI executable. Default relies on Buzz-injected PATH (.local/bin first). */
  cliPath: z.string().default('buzz'),
  /** Wall-clock cap for one publish subprocess. */
  timeoutMs: z.number().default(120_000),
})

/**
 * Run one `buzz messages send` invocation.
 * @param {string} command - CLI executable to spawn.
 * @param {string[]} argv - full argument vector after the executable.
 * @param {number} timeoutMs - kill the subprocess after this many ms.
 * @returns {Promise<{stdout: string, stderr: string}>} raw streams on exit 0.
 */
function runCli(command, argv, timeoutMs) {
  return new Promise((resolve, reject) => {
    // Explicit env layer: the inherited process environment plus the two
    // credential names restated. Restating documents that they are deliberate
    // at THIS boundary (upstream scrub applies to tool children, not here) and
    // matches upstream's explicit-entry-merges-after-scrub contract.
    const env = { ...process.env }
    if (process.env.BUZZ_PRIVATE_KEY !== undefined) env.BUZZ_PRIVATE_KEY = process.env.BUZZ_PRIVATE_KEY
    if (process.env.BUZZ_AUTH_TAG !== undefined) env.BUZZ_AUTH_TAG = process.env.BUZZ_AUTH_TAG
    execFile(command, argv, { timeout: timeoutMs, maxBuffer: 4 * 1024 * 1024, encoding: 'utf8', env }, (error, stdout, stderr) => {
      if (error) {
        const detail = String(stderr || stdout || '').trim()
        reject(new Error(`buzz messages send failed (${error.code ?? error.signal ?? 'unknown'}): ${detail || error.message}`))
        return
      }
      resolve({ stdout, stderr })
    })
  })
}

/** Last successful publish per agent session id — the duplicate-publish guard state. */
const lastPublish = new Map()

/** Test seam: reset guard state and swap the CLI runner. Exported for tests only. */
export const _internals = {
  reset() {
    lastPublish.clear()
  },
  setRunner(runner) {
    internalsRunner = runner
  },
}

let internalsRunner = runCli

/**
 * Stable fingerprint of one publish request (guard input).
 * @param {{channel: string, content: string, reply_to?: string}} args - validated tool args.
 * @returns {string} fingerprint key.
 */
function fingerprint(args) {
  return JSON.stringify([args.channel.trim(), args.content.trim(), args.reply_to?.trim() ?? ''])
}

/**
 * Register the `buzz_publish` tool on `ctx.tools`.
 * @param {import('@deepseek-ai/cordis').Context} ctx - registrant context carrying the tool registry.
 * @param {{cliPath: string, timeoutMs: number}} config - resolved deployment config.
 */
export function apply(ctx, config) {
  ctx.tools.register(defineTool({
    name: 'buzz_publish',
    description:
      "Publish a chat message to a Buzz channel under this agent's own identity. "
      + 'Use it whenever your reply must be visible in a Buzz channel thread — answering a '
      + "mention, reporting finished work or blockers, or posting a milestone. In Buzz, a turn "
      + 'that ends without a publication counts as chosen silence, so deliver substantive '
      + 'results through this tool rather than only as final text. '
      + '`channel` is the Buzz channel UUID; `content` is the message body (GitHub-flavored '
      + 'Markdown); optionally set `reply_to` to the 64-char hex event id of the thread root or '
      + 'message you are answering, and `mentions` to a list of pubkeys or npubs that must be '
      + 'notified (e.g. the person who delegated the work). Publish each distinct message ONCE '
      + '— repeating an identical publish returns the prior event id instead of sending again. '
      + 'Returns the accepted flag and the published event id so you can cite it later.',
    parameters: {
      channel: {
        type: 'string',
        required: true,
        description: 'Buzz channel UUID to publish into.',
      },
      content: {
        type: 'string',
        required: true,
        description: 'Message body. GitHub-flavored Markdown.',
      },
      reply_to: {
        type: 'string',
        description: 'Optional 64-char hex event id to reply to (thread root or target message).',
      },
      mentions: {
        type: 'array',
        description: 'Optional pubkeys/npubs to @notify. Use only when someone must act or be credited.',
        items: { type: 'string' },
      },
    },
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          accepted: { type: 'boolean', required: true },
          event_id: { type: 'string' },
          mention_pubkeys: { type: 'array', items: { type: 'string' } },
          raw: { type: 'string' },
          duplicate_of_prior_publish: { type: 'boolean' },
        },
      },
      render: (_args, value) => [{
        type: 'text',
        text: value.duplicate_of_prior_publish === true
          ? `Not sent again — identical message already published as ${value.event_id}.`
          : value.event_id
            ? `Published event ${value.event_id}${Array.isArray(value.mention_pubkeys) && value.mention_pubkeys.length > 0 ? ` (notified ${value.mention_pubkeys.length})` : ''}.`
            : `Publish result: ${String(value.raw ?? '').slice(0, 400)}`,
      }],
    },
    async execute(args, exec) {
      const channel = args.channel.trim()
      const content = args.content.trim()
      if (channel.length === 0) throw new Error('invalid arguments: `channel` must be a non-empty Buzz channel UUID')
      if (content.length === 0) throw new Error('invalid arguments: `content` must be a non-empty message body')

      const sessionKey = exec?.agent ? String(exec.agent.session.header.id) : '(no-session)'
      const print = fingerprint({ channel, content, reply_to: args.reply_to })
      const prior = lastPublish.get(sessionKey)
      if (prior && prior.print === print) {
        return { accepted: true, event_id: prior.eventId, duplicate_of_prior_publish: true }
      }

      if (!process.env.BUZZ_PRIVATE_KEY) {
        throw new Error('buzz_publish unavailable: BUZZ_PRIVATE_KEY is not set in the harness environment')
      }
      const argv = ['messages', 'send', '--channel', channel, '--content', content]
      if (args.reply_to !== undefined && args.reply_to.trim().length > 0) argv.push('--reply-to', args.reply_to.trim())
      for (const mention of args.mentions ?? []) {
        const value = String(mention).trim()
        if (value.length > 0) argv.push('--mention', value)
      }
      const { stdout } = await internalsRunner(config.cliPath, argv, config.timeoutMs)
      let result
      try {
        const receipt = JSON.parse(stdout)
        result = {
          accepted: receipt.accepted === true,
          ...(receipt.event_id !== undefined ? { event_id: String(receipt.event_id) } : {}),
          ...(Array.isArray(receipt.mention_pubkeys) ? { mention_pubkeys: receipt.mention_pubkeys.map(String) } : {}),
        }
      } catch {
        // CLI printed non-JSON (banner, warning, or older format) — hand the raw text back.
        result = { accepted: true, raw: stdout.trim() }
      }
      if (result.accepted && typeof result.event_id === 'string') {
        lastPublish.set(sessionKey, { print, eventId: result.event_id })
      }
      return result
    },
    presentCall: args => ({ card: 'generic', title: 'Publish to Buzz channel', kind: 'other', rawInput: args }),
  }))
}
