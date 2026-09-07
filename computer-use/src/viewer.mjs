import { createServer } from "node:http";
import { randomBytes, timingSafeEqual } from "node:crypto";
import { readFile } from "node:fs/promises";

const html = await readFile(new URL("../viewer/index.html", import.meta.url));
const script = await readFile(new URL("../viewer/viewer.js", import.meta.url));
const style = await readFile(new URL("../viewer/viewer.css", import.meta.url));

/** Private loopback viewer. Its human-only token never appears in agent tool results. */
export async function startViewer(browser) {
  const token = randomBytes(32).toString("hex");
  let origin;
  const server = createServer(async (req, res) => {
    res.setHeader("Cache-Control", "no-store");
    res.setHeader("Referrer-Policy", "no-referrer");
    res.setHeader("X-Content-Type-Options", "nosniff");
    res.setHeader(
      "Content-Security-Policy",
      "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors tauri: http://tauri.localhost https://tauri.localhost http://localhost:* http://127.0.0.1:*",
    );
    const reply = (status, value) => {
      res.writeHead(status, { "Content-Type": "application/json" });
      res.end(JSON.stringify(value));
    };
    try {
      if (req.headers.host !== new URL(origin).host)
        return reply(403, { error: "Invalid host" });
      if (req.headers.origin && req.headers.origin !== origin)
        return reply(403, { error: "Invalid origin" });
      const assets = {
        "/": [html, "text/html"],
        "/viewer.js": [script, "text/javascript"],
        "/viewer.css": [style, "text/css"],
      };
      if (req.method === "GET" && assets[req.url]) {
        res.writeHead(200, { "Content-Type": assets[req.url][1] });
        return res.end(assets[req.url][0]);
      }
      const auth = Buffer.from(
        req.headers.authorization?.replace(/^Bearer /, "") || "",
      );
      if (
        auth.length !== token.length ||
        !timingSafeEqual(auth, Buffer.from(token))
      )
        return reply(401, { error: "Viewer authorization required" });
      if (req.method !== "POST" || req.url !== "/control")
        return reply(404, { error: "Unknown endpoint" });
      let body = "";
      for await (const chunk of req) {
        body += chunk;
        if (body.length > 64_000)
          return reply(413, { error: "Request too large" });
      }
      const data = JSON.parse(body);
      let result;
      switch (data.op) {
        case "list":
          result = browser.list();
          break;
        case "open":
          result = await browser.open(data.url);
          break;
        case "frame": {
          result = await browser.frame(data.id);
          result.image = result.image.toString("base64");
          break;
        }
        case "control":
          result = await browser.control(data.id, data.controller);
          break;
        case "action":
          result = await browser.act(data.id, data.action, "human");
          break;
        case "close":
          await browser.close(data.id);
          result = { closed: true };
          break;
        default:
          return reply(400, { error: "Unknown operation" });
      }
      reply(200, result);
    } catch (error) {
      reply(400, { error: error.message });
    }
  });
  server.requestTimeout = 25_000;
  server.headersTimeout = 5000;
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  origin = `http://127.0.0.1:${server.address().port}`;
  return {
    url: `${origin}/#${token}`,
    close: async () => {
      server.closeAllConnections();
      await new Promise((resolve) => server.close(resolve));
    },
  };
}
