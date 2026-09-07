const token = location.hash.slice(1);
history.replaceState(null, "", location.pathname);
const $ = (id) => document.getElementById(id);
let selected = null;
let frame = null;
let stopped = false;
let lastList = "";
let inputQueue = Promise.resolve();
let revision = 0;
const api = async (data) => {
  const response = await fetch("/control", {
    method: "POST",
    headers: {
      Authorization: `Bearer ${token}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify(data),
  });
  const value = await response.json();
  if (!response.ok) throw new Error(value.error || "Browser unavailable");
  return value;
};
const report = (error) => {
  $("status").textContent = error.message;
  $("status").className = "error";
};
function action(data) {
  const id = selected;
  const result = inputQueue.then(async () => {
    if (id !== selected || !frame || frame.controller !== "human")
      throw new Error("Take control before interacting");
    revision++;
    frame = await api({
      op: "action",
      id,
      action: { ...data, generation: frame.generation },
    });
  });
  inputQueue = result.catch(() => {});
  return result;
}
function run(fn) {
  return (event) => {
    event?.preventDefault();
    Promise.resolve()
      .then(() => fn(event))
      .catch(report);
  };
}
$("new").onclick = run(async () => {
  const s = await api({ op: "open" });
  selected = s.id;
  await api({ op: "control", id: selected, controller: "human" });
});
$("control").onclick = run(async () => {
  if (!frame) return;
  revision++;
  await inputQueue;
  $("control").disabled = true;
  try {
    frame = await api({
      op: "control",
      id: selected,
      controller: frame.controller === "human" ? "agent" : "human",
    });
  } finally {
    $("control").disabled = false;
  }
  await fitViewport();
});
$("close").onclick = run(async () => {
  if (selected) await api({ op: "close", id: selected });
  selected = null;
  frame = null;
});
$("navigation").onsubmit = run(() =>
  action({ kind: "navigate", url: $("url").value }),
);
$("back").onclick = run(() => action({ kind: "back" }));
$("reload").onclick = run(() => action({ kind: "reload" }));
$("typing").onsubmit = run(async () => {
  await action({ kind: "type", text: $("text").value });
  $("text").value = "";
});
$("page").onclick = run((event) => {
  const rect = $("page").getBoundingClientRect();
  return action({
    kind: "click",
    x: ((event.clientX - rect.left) * frame.viewport.width) / rect.width,
    y: ((event.clientY - rect.top) * frame.viewport.height) / rect.height,
  });
});
$("page").onkeydown = run((event) => {
  if (event.isComposing) return;
  const modifiers = [
    event.ctrlKey && "Control",
    event.altKey && "Alt",
    event.metaKey && "Meta",
    event.shiftKey && "Shift",
  ].filter(Boolean);
  if (!modifiers.length && event.key.length === 1)
    return action({ kind: "type", text: event.key });
  if (["Shift", "Control", "Alt", "Meta"].includes(event.key)) return;
  return action({
    kind: "key",
    key: [...modifiers, event.key === " " ? "Space" : event.key].join("+"),
  });
});
$("page").addEventListener(
  "wheel",
  run((event) =>
    action({
      kind: "scroll",
      dy: Math.max(-4000, Math.min(4000, event.deltaY)),
    }),
  ),
  { passive: false },
);
async function fitViewport() {
  if (frame?.controller !== "human") return;
  const area = document.querySelector("main").getBoundingClientRect();
  const width = Math.max(320, Math.min(1920, Math.floor(area.width)));
  const height = Math.max(200, Math.min(1200, Math.floor(area.height)));
  if (frame.viewport.width !== width || frame.viewport.height !== height)
    await action({ kind: "resize", width, height });
}
new ResizeObserver(() => {
  void fitViewport().catch(report);
}).observe(document.querySelector("main"));
async function refresh() {
  try {
    const list = await api({ op: "list" });
    if (!list.some((s) => s.id === selected)) {
      selected = list[0]?.id ?? null;
      frame = null;
    }
    const key = JSON.stringify(
      list.map((s) => [s.id, s.url, s.controller, selected]),
    );
    if (key !== lastList) {
      $("sessions").replaceChildren(
        ...list.map((s, index) => {
          const button = document.createElement("button");
          button.textContent = `Session ${index + 1} · ${s.controller}`;
          button.setAttribute("aria-current", String(s.id === selected));
          button.title = s.url;
          button.onclick = () => {
            selected = s.id;
            frame = null;
          };
          return button;
        }),
      );
      lastList = key;
    }
    $("empty").hidden = !!selected;
    $("page").style.display = selected ? "block" : "none";
    if (selected) {
      const id = selected;
      const expected = revision;
      const next = await api({ op: "frame", id });
      if (selected !== id || revision !== expected) return;
      frame = next;
      $("page").src = `data:image/jpeg;base64,${next.image}`;
      if (document.activeElement !== $("url")) $("url").value = next.url;
      $("control").textContent =
        next.controller === "human" ? "Return to agent" : "Take control";
      $("status").textContent =
        `${next.controller === "human" ? "You control this page" : "Agent controls this page"} · Live local view${next.dialog ? ` · Dialog: ${next.dialog.message}` : ""}`;
    } else $("status").textContent = "No browser sessions";
    $("status").className = "";
  } catch (error) {
    report(error);
  } finally {
    if (!stopped) setTimeout(refresh, 500);
  }
}
window.addEventListener("pagehide", () => {
  stopped = true;
});
void refresh();
