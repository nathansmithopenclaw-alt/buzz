import { expect, test } from "@playwright/test";

import type { RelayEvent } from "@/shared/api/types";
import { installMockBridge } from "../helpers/bridge";

const OWNER = "deadbeef".repeat(8);
const AGENT = "2".repeat(64);
const SECOND_AGENT = "5".repeat(64);
const HOST = "018f42c2-9bc2-7dc1-a41e-2f77769f0142";
const SECOND_HOST = "018f42c2-9bc2-7dc1-a41e-2f77769f0144";
const TEAM_OWNER =
  "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f";
const TEAM_AGENT = "4".repeat(64);
const TEAM_HOST = "018f42c2-9bc2-7dc1-a41e-2f77769f0143";
const OUTSIDER = "6".repeat(64);

function projectionEvent(
  kind: number,
  content: Record<string, unknown>,
  dTag: string,
  createdAt: number,
  idDigit: string,
  pubkey = OWNER,
): RelayEvent {
  return {
    id: idDigit.repeat(64),
    pubkey,
    created_at: createdAt,
    kind,
    tags: [["d", dTag]],
    content: JSON.stringify(content),
    sig: "f".repeat(128),
  };
}

test("Observer renders relay fleet liveness and mentions the existing agent", async ({
  page,
}) => {
  const now = Math.floor(Date.now() / 1_000);
  await installMockBridge(page, {
    fleetVisibilitySettings: {
      enabled: true,
      role: "observer",
      needsChoice: false,
      hostId: HOST,
      hostName: "Observer device",
    },
    managedAgents: [
      {
        pubkey: "3".repeat(64),
        name: "Stale Local Agent",
        status: "stopped",
      },
    ],
    personas: [
      {
        id: "will-create",
        displayName: "Will Create Persona",
        systemPrompt: "Definition only",
        isActive: true,
      },
    ],
    teams: [
      {
        id: "will-create-team",
        name: "Will Create Team",
        personaIds: ["will-create"],
      },
    ],
    fleetProjectionEvents: [
      projectionEvent(
        30177,
        {
          name: "Fallback Review Agent",
          persona_id: "reviewer",
          parallelism: 1,
          respond_to: "owner-only",
          respond_to_allowlist: [],
        },
        AGENT,
        now - 30,
        "1",
      ),
      projectionEvent(
        30175,
        {
          display_name: "Review Agent",
          system_prompt: "Review carefully.",
          avatar_url: null,
          runtime: "codex",
          model: "gpt-5",
          provider: null,
          name_pool: [],
        },
        "reviewer",
        now - 29,
        "2",
      ),
      projectionEvent(
        30180,
        {
          v: 1,
          name: "Mini host",
          platform: "macos",
          created_at: now - 120,
        },
        HOST,
        now - 28,
        "3",
      ),
      projectionEvent(
        30181,
        {
          v: 1,
          seq: 7,
          ts: now - 2,
          agents: [{ pubkey: AGENT, state: "running", since: now - 40 }],
        },
        HOST,
        now - 2,
        "4",
      ),
      projectionEvent(
        30177,
        {
          name: "Quiet Operator",
          persona_id: "operator",
          parallelism: 1,
          respond_to: "owner-only",
          respond_to_allowlist: [],
        },
        SECOND_AGENT,
        now - 27,
        "9",
      ),
      projectionEvent(
        30175,
        {
          display_name: "Quiet Operator",
          system_prompt: "Operate carefully.",
          avatar_url: null,
          runtime: "claude",
          model: "claude-sonnet",
          provider: null,
          name_pool: [],
        },
        "operator",
        now - 26,
        "a",
      ),
      projectionEvent(
        30180,
        {
          v: 1,
          name: "Backup host",
          platform: "macos",
          created_at: now - 120,
        },
        SECOND_HOST,
        now - 25,
        "b",
      ),
      projectionEvent(
        30181,
        {
          v: 1,
          seq: 2,
          ts: now - 4,
          agents: [{ pubkey: SECOND_AGENT, state: "stopped", since: now - 50 }],
        },
        SECOND_HOST,
        now - 4,
        "c",
      ),
      projectionEvent(
        30175,
        {
          display_name: "Standalone Definition",
          system_prompt: "Definition only.",
          avatar_url: null,
          runtime: "goose",
          model: "openai/gpt-5",
          provider: null,
          name_pool: [],
        },
        "standalone",
        now - 23,
        "d",
      ),
      projectionEvent(
        30177,
        {
          name: "Team Review Agent",
          persona_id: null,
          parallelism: 1,
          respond_to: "anyone",
          respond_to_allowlist: [],
        },
        TEAM_AGENT,
        now - 25,
        "5",
        TEAM_OWNER,
      ),
      projectionEvent(
        30180,
        {
          v: 1,
          name: "Team host",
          platform: "macos",
          created_at: now - 120,
        },
        TEAM_HOST,
        now - 24,
        "6",
        TEAM_OWNER,
      ),
      projectionEvent(
        30181,
        {
          v: 1,
          seq: 3,
          ts: now - 3,
          agents: [{ pubkey: TEAM_AGENT, state: "stopped", since: now - 50 }],
        },
        TEAM_HOST,
        now - 3,
        "7",
        TEAM_OWNER,
      ),
      projectionEvent(
        30177,
        {
          name: "Outsider Agent",
          persona_id: null,
          parallelism: 1,
          respond_to: "anyone",
          respond_to_allowlist: [],
        },
        "8".repeat(64),
        now - 20,
        "e",
        OUTSIDER,
      ),
    ],
  });

  await page.goto("/");
  await page.getByTestId("open-agents-view").click();
  await expect(page.getByTestId("agents-fleet")).toBeVisible();

  const orderedSections = await page
    .getByTestId("agents-page-content")
    .locator("section")
    .evaluateAll((sections) =>
      sections
        .map((section) => section.getAttribute("data-testid"))
        .filter((testId) =>
          [
            "agents-fleet",
            "agents-library-personas",
            "agents-library-teams",
          ].includes(testId ?? ""),
        ),
    );
  expect(orderedSections).toEqual([
    "agents-fleet",
    "agents-library-personas",
    "agents-library-teams",
  ]);
  await expect(page.getByTestId("fleet-host-status")).toContainText(
    "Mini host",
  );
  await expect(page.getByTestId("fleet-host-status")).toContainText(
    "Backup host",
  );
  await expect(page.getByTestId(`fleet-agent-${AGENT}`)).toContainText(
    "Review Agent",
  );
  await expect(page.getByTestId(`fleet-agent-${AGENT}`)).toContainText(
    "codex · gpt-5",
  );
  await expect(page.getByTestId(`fleet-agent-${AGENT}`)).toContainText("live");
  await expect(page.getByTestId(`fleet-agent-${SECOND_AGENT}`)).toContainText(
    "quiet · last seen",
  );
  await expect(page.getByTestId("agents-fleet")).toContainText("Mini host");
  await expect(page.getByTestId("agents-fleet")).toContainText("Backup host");
  await expect(page.getByTestId("agents-library-personas")).toContainText(
    "Standalone Definition",
  );
  await expect(page.getByTestId("agents-library-personas")).toContainText(
    "goose · openai/gpt-5",
  );
  await expect(page.getByTestId("fleet-team-rosters")).toContainText(
    "Team Review Agent",
  );
  await expect(page.getByText("Outsider Agent", { exact: true })).toHaveCount(
    0,
  );
  await expect(page.getByTestId("new-agent-card")).toHaveCount(0);
  await expect(page.getByTestId("new-team-card")).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Browse catalog" }),
  ).toHaveCount(0);
  await expect(page.getByRole("button", { name: /^Deploy / })).toHaveCount(0);
  await expect(
    page.getByText("Stale Local Agent", { exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByText("Will Create Persona", { exact: true }),
  ).toHaveCount(0);
  await expect(page.getByText("Will Create Team", { exact: true })).toHaveCount(
    0,
  );

  await page
    .getByTestId(`fleet-agent-${AGENT}`)
    .getByRole("button", { name: "Open actions for Review Agent" })
    .click();
  await expect(page.getByRole("menuitem", { name: "Message" })).toBeVisible();
  await expect(page.getByRole("menuitem", { name: "Start" })).toHaveCount(0);
  await expect(page.getByRole("menuitem", { name: "Stop" })).toHaveCount(0);
  await expect(page.getByRole("menuitem", { name: "Restart" })).toHaveCount(0);
  await page.keyboard.press("Escape");

  await page.getByTestId("channel-general").click();
  const composer = page.getByTestId("message-input");
  await composer.fill("@Review");
  const autocomplete = page
    .getByTestId("message-composer")
    .getByTestId("mention-autocomplete");
  await expect(
    autocomplete.getByText("Review Agent", { exact: true }),
  ).toBeVisible();
  await expect(
    autocomplete.getByText("Will Create Persona", { exact: true }),
  ).toHaveCount(0);
  await expect(
    autocomplete.getByText("Will Create Team", { exact: true }),
  ).toHaveCount(0);

  await composer.fill("@Stale");
  await expect(
    autocomplete.getByText("Stale Local Agent", { exact: true }),
  ).toHaveCount(0);

  await page.reload();
  await page.getByTestId("open-agents-view").click();
  await expect(page.getByTestId(`fleet-agent-${AGENT}`)).toContainText(
    "Review Agent",
  );
  await expect(page.getByTestId("fleet-host-status")).toContainText(
    "Mini host",
  );
});

test("Observer greys liveness after the 90-second freshness boundary", async ({
  page,
}) => {
  const now = Math.floor(Date.now() / 1_000);
  await installMockBridge(page, {
    fleetVisibilitySettings: {
      enabled: true,
      role: "observer",
      needsChoice: false,
    },
    fleetProjectionEvents: [
      projectionEvent(
        30177,
        {
          name: "Stale Fleet Agent",
          persona_id: null,
          parallelism: 1,
          respond_to: "owner-only",
          respond_to_allowlist: [],
        },
        AGENT,
        now - 120,
        "9",
      ),
      projectionEvent(
        30180,
        {
          v: 1,
          name: "Offline host",
          platform: "macos",
          created_at: now - 180,
        },
        HOST,
        now - 119,
        "a",
      ),
      projectionEvent(
        30181,
        {
          v: 1,
          seq: 8,
          ts: now - 91,
          agents: [{ pubkey: AGENT, state: "running", since: now - 180 }],
        },
        HOST,
        now - 91,
        "b",
      ),
    ],
  });

  await page.goto("/");
  await page.getByTestId("open-agents-view").click();

  const hostStatus = page.getByTestId("fleet-host-status");
  await expect(hostStatus).toHaveAttribute("data-fresh", "false");
  await expect(hostStatus).toContainText("Host unreachable · last seen");
  await expect(page.getByTestId(`fleet-agent-${AGENT}`)).toContainText(
    "host unreachable · last seen",
  );
});

test("Observer explains when no host is publishing without inventing status", async ({
  page,
}) => {
  const now = Math.floor(Date.now() / 1_000);
  await installMockBridge(page, {
    fleetVisibilitySettings: {
      enabled: true,
      role: "observer",
      needsChoice: false,
    },
    fleetProjectionEvents: [
      projectionEvent(
        30177,
        {
          name: "Unreported Agent",
          persona_id: null,
          parallelism: 1,
          respond_to: "owner-only",
          respond_to_allowlist: [],
        },
        AGENT,
        now - 20,
        "c",
      ),
    ],
  });

  await page.goto("/");
  await page.getByTestId("open-agents-view").click();

  await expect(page.getByTestId("fleet-host-status")).toContainText(
    "No fleet host is publishing right now",
  );
  await expect(page.getByTestId(`fleet-agent-${AGENT}`)).toContainText(
    "quiet · last seen not reported",
  );
});

test("pending role choice hides legacy local state behind the designed banner", async ({
  page,
}) => {
  const now = Math.floor(Date.now() / 1_000);
  await installMockBridge(page, {
    fleetVisibilitySettings: {
      enabled: true,
      role: "host",
      needsChoice: true,
    },
    managedAgents: [
      {
        pubkey: "3".repeat(64),
        name: "Legacy Local Ghost",
        status: "stopped",
      },
    ],
    personas: [
      {
        id: "legacy-definition",
        displayName: "Legacy Local Definition",
        systemPrompt: "Do not render this before role choice.",
      },
    ],
    fleetProjectionEvents: [
      projectionEvent(
        30177,
        {
          name: "Remote Fleet Agent",
          persona_id: null,
          parallelism: 1,
          respond_to: "owner-only",
          respond_to_allowlist: [],
        },
        AGENT,
        now - 20,
        "d",
      ),
    ],
  });

  await page.goto("/");
  await page
    .getByTestId("open-agents-view")
    .evaluate((button: HTMLButtonElement) => button.click());

  await expect(page.getByTestId("fleet-role-choice-banner")).toBeVisible();
  await expect(
    page.getByText("Legacy Local Ghost", { exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByText("Legacy Local Definition", { exact: true }),
  ).toHaveCount(0);
  await expect(page.getByTestId("agents-fleet")).toHaveCount(0);
  await expect(page.getByTestId("agents-library-personas")).toHaveCount(0);
  await expect(page.getByTestId("agents-library-teams")).toHaveCount(0);
});
