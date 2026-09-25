import { invokeTauri } from "@/shared/api/tauri";
const connectionListeners = new Set<() => void>();
export function subscribeConnections(listener: () => void) {
  connectionListeners.add(listener);
  return () => {
    connectionListeners.delete(listener);
  };
}
async function notifyConnectionChange<T>(write: Promise<T>): Promise<T> {
  const result = await write;
  for (const listener of connectionListeners) listener();
  return result;
}

export const CONNECTION_ENV = "BUZZ_MODEL_CONNECTION_ID";
export type ModelConnection = {
  id: string;
  name: string;
  base_url: string;
  models: string[];
  has_key: boolean;
  tested_model: string | null;
  tested_at: number | null;
  qualification?: {
    revision: number;
    model: string;
    context_window: number | null;
    output_token_limit: number | null;
    protocols: {
      protocol: "chat_completions" | "responses" | "anthropic_messages";
      tool_round_trip: boolean;
      reason: string | null;
      effort_levels: string[];
    }[];
  } | null;
};
export type DiscoveredServer = {
  name: string;
  base_url: string;
  models: string[];
  needs_key: boolean;
};
export const listConnections = () =>
  invokeTauri<ModelConnection[]>("list_model_connections");
type DiscoveryResult = { servers: DiscoveredServer[]; notice: string };
const pendingDiscovery = new Map<boolean, Promise<DiscoveryResult>>();
export function discoverServers(
  includePrivateNetwork = false,
): Promise<DiscoveryResult> {
  let pending = pendingDiscovery.get(includePrivateNetwork);
  if (!pending) {
    pending = invokeTauri<DiscoveryResult>("discover_model_servers", {
      includePrivateNetwork,
    }).finally(() => {
      pendingDiscovery.delete(includePrivateNetwork);
    });
    pendingDiscovery.set(includePrivateNetwork, pending);
  }
  return pending;
}
export const saveConnection = (input: {
  id?: string;
  name: string;
  base_url: string;
  api_key?: string;
}) =>
  notifyConnectionChange(
    invokeTauri<ModelConnection>("save_model_connection", { input }),
  );
export const removeConnection = (id: string) =>
  notifyConnectionChange(invokeTauri<void>("remove_model_connection", { id }));
export const testConnection = (id: string, model: string) =>
  notifyConnectionChange(
    invokeTauri<ModelConnection>("test_model_connection", { id, model }),
  );
