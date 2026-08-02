import {
  getAgentWorkingState,
  subscribeAgentWorkingSignal,
} from "./agentWorkingSignal";
import {
  noteClaudeAccountSwitchQueued,
  switchAgentClaudeAccount,
  type ClaudeAccountSwitchResult,
} from "@/shared/api/tauriClaudeAccounts";

export type ClaudeAccountSwitchState =
  | { status: "queued"; accountId: string | null }
  | { status: "applying"; accountId: string | null }
  | { status: "complete"; result: ClaudeAccountSwitchResult }
  | { status: "failed"; error: string };

const switches = new Map<string, ClaudeAccountSwitchState>();
const listeners = new Map<
  string,
  Set<(state: ClaudeAccountSwitchState | null) => void>
>();
let _unsubscribeWorking: (() => void) | null = null;

function emit(pubkey: string) {
  const state = switches.get(pubkey) ?? null;
  for (const listener of listeners.get(pubkey) ?? []) listener(state);
}

async function drainReadySwitches() {
  for (const [pubkey, state] of switches) {
    if (state.status !== "queued" || getAgentWorkingState(pubkey).working) {
      continue;
    }
    switches.set(pubkey, { ...state, status: "applying" });
    emit(pubkey);
    try {
      const result = await switchAgentClaudeAccount(
        pubkey,
        state.accountId,
        true,
      );
      switches.set(pubkey, { status: "complete", result });
    } catch (cause) {
      switches.set(pubkey, { status: "failed", error: String(cause) });
    }
    emit(pubkey);
  }
}

function ensureWorkingSubscription() {
  _unsubscribeWorking ??= subscribeAgentWorkingSignal(() => {
    void drainReadySwitches();
  });
}

export function queueClaudeAccountSwitch(
  pubkey: string,
  accountId: string | null,
) {
  switches.set(pubkey, { status: "queued", accountId });
  ensureWorkingSubscription();
  emit(pubkey);
  void noteClaudeAccountSwitchQueued(pubkey).catch((cause) => {
    switches.set(pubkey, { status: "failed", error: String(cause) });
    emit(pubkey);
  });
  void drainReadySwitches();
}

export function subscribeClaudeAccountSwitch(
  pubkey: string,
  listener: (state: ClaudeAccountSwitchState | null) => void,
) {
  const entries = listeners.get(pubkey) ?? new Set();
  entries.add(listener);
  listeners.set(pubkey, entries);
  listener(switches.get(pubkey) ?? null);
  return () => {
    entries.delete(listener);
    if (entries.size === 0) listeners.delete(pubkey);
  };
}

export function clearClaudeAccountSwitch(pubkey: string) {
  switches.delete(pubkey);
  emit(pubkey);
}
